//! Page /conformite : bonnes pratiques HDS vérifiées en direct, sans aucune donnée patient.

use std::sync::Arc;

use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::Html;
use axum::Json;
use serde::Serialize;

use crate::auth::{self, attributs_cookie, nom_cookie};
use crate::{masquer_chemin, App, EN_TETES};

#[derive(Serialize, Clone, Copy, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Statut {
    Ok,
    Ko,
    Info,
}

#[derive(Serialize)]
pub struct Verification {
    pub id: &'static str,
    pub libelle: String,
    pub valeur: String,
    pub statut: Statut,
}

#[derive(Serialize)]
pub struct Section {
    pub titre: &'static str,
    pub verifications: Vec<Verification>,
}

#[derive(Serialize, Default)]
pub struct Synthese {
    pub ok: usize,
    pub ko: usize,
    pub info: usize,
}

/// Connexion de CETTE requête à PostgreSQL, lue dans pg_stat_ssl (lue par scripts/verif-hds.sh).
#[derive(Serialize, Default)]
pub struct Base {
    pub joignable: bool,
    pub ssl: bool,
    pub version_tls: Option<String>,
    pub chiffrement: Option<String>,
    pub bits: Option<i32>,
    pub postgresql: Option<String>,
}

#[derive(Serialize)]
pub struct PointPlateforme {
    pub point: &'static str,
    pub verification: &'static str,
}

#[derive(Serialize)]
pub struct Conformite {
    pub genere_le: String,
    pub mode: String,
    pub synthese: Synthese,
    pub base: Base,
    pub sections: Vec<Section>,
    pub plateforme: Vec<PointPlateforme>,
}

fn v(id: &'static str, libelle: &str, valeur: String, statut: Statut) -> Verification {
    Verification {
        id,
        libelle: libelle.into(),
        valeur,
        statut,
    }
}

fn ok_si(condition: bool) -> Statut {
    if condition {
        Statut::Ok
    } else {
        Statut::Ko
    }
}

pub const PLATEFORME: [PointPlateforme; 6] = [
    PointPlateforme { point: "Zone HDS parhds", verification: "Application et add-on PostgreSQL dans la zone parhds (tag contract:hds de l'API zones)." },
    PointPlateforme { point: "Chiffrement au repos", verification: "Option encryption de l'add-on PostgreSQL, lue dans l'API v4 addon-providers. Choisie à la création, définitive." },
    PointPlateforme { point: "Sauvegardes", verification: "Plan dédié avec sauvegardes quotidiennes (7 conservées) et option do-backup active." },
    PointPlateforme { point: "HTTPS forcé", verification: "Réglage force-https de l'application, puis curl : http → 301 vers https et en-tête HSTS." },
    PointPlateforme { point: "Double authentification", verification: "Tous les membres de l'organisation ont la 2FA active (API organisations/members)." },
    PointPlateforme { point: "Contrat HDS", verification: "Contrat HDS signé avec Clever Cloud avant toute donnée réelle : non lisible par l'API, rappelé par le script." },
];

pub async fn calculer(app: &App, headers: &HeaderMap) -> Conformite {
    let maintenant: String =
        sqlx::query_scalar("SELECT to_char(now() AT TIME ZONE 'Europe/Paris', 'DD/MM/YYYY HH24:MI:SS')")
            .fetch_one(&app.pool)
            .await
            .unwrap_or_else(|_| "base injoignable".into());

    // 1. Base de données : la connexion de cette requête est-elle chiffrée ?
    type LigneSsl = (bool, Option<String>, Option<String>, Option<i32>, String);
    let ligne: Option<LigneSsl> = sqlx::query_as(
        "SELECT s.ssl, s.version, s.cipher, s.bits, current_setting('server_version')
         FROM pg_stat_ssl s WHERE s.pid = pg_backend_pid()",
    )
    .fetch_optional(&app.pool)
    .await
    .ok()
    .flatten();
    let base = match ligne {
        Some((ssl, version_tls, chiffrement, bits, pg)) => Base {
            joignable: true,
            ssl,
            version_tls,
            chiffrement,
            bits,
            postgresql: Some(pg),
        },
        None => Base::default(),
    };
    let tls_detail = match (&base.version_tls, &base.chiffrement, base.bits) {
        (Some(v), Some(c), Some(b)) => format!("{v} · {c} · {b} bits"),
        _ if !base.joignable => "base injoignable".into(),
        _ => "connexion en clair".into(),
    };
    let base_section = Section {
        titre: "Base de données",
        verifications: vec![
            v(
                "base.tls",
                "Connexion à PostgreSQL chiffrée (pg_stat_ssl)",
                tls_detail,
                ok_si(base.ssl),
            ),
            v(
                "base.version",
                "Version de PostgreSQL",
                base.postgresql.clone().unwrap_or_else(|| "inconnue".into()),
                if base.joignable { Statut::Info } else { Statut::Ko },
            ),
            v(
                "base.mode_tls",
                "TLS exigé par l'application (sslmode)",
                if app.tls_base {
                    "require : refus d'une connexion en clair".into()
                } else {
                    "disable (PostgreSQL local uniquement)".into()
                },
                if app.tls_base {
                    Statut::Ok
                } else if app.production {
                    Statut::Ko
                } else {
                    Statut::Info
                },
            ),
        ],
    };

    // 2. HTTPS : cette requête est-elle arrivée en HTTPS ?
    let proto = headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("absent")
        .to_string();
    let https = proto == "https";
    let https_section = Section {
        titre: "HTTPS",
        verifications: vec![
            v(
                "https.requete",
                "Requête reçue en HTTPS (X-Forwarded-Proto)",
                proto.clone(),
                if https {
                    Statut::Ok
                } else if app.production {
                    Statut::Ko
                } else {
                    Statut::Info
                },
            ),
            v(
                "https.redirection",
                "Redirection HTTP → HTTPS par l'application (301, sauf /health)",
                if app.production {
                    "active (APP_ENV=production)".into()
                } else {
                    "inactive en local".into()
                },
                if app.production { Statut::Ok } else { Statut::Info },
            ),
        ],
    };

    // 3. En-têtes de sécurité : la constante même qu'applique le middleware à chaque réponse.
    let valeur = |nom: &str| EN_TETES.iter().find(|(n, _)| *n == nom).map(|(_, v)| v.to_string());
    let attendu = |nom: &str, motif: &str| valeur(nom).filter(|v| v.contains(motif));
    let en_tete = |id, libelle: &str, nom: &str, motif: &str| {
        let trouve = attendu(nom, motif);
        v(
            id,
            libelle,
            trouve.clone().unwrap_or_else(|| "absent".into()),
            ok_si(trouve.is_some()),
        )
    };
    let en_tetes_section = Section {
        titre: "En-têtes de sécurité envoyés",
        verifications: vec![
            en_tete(
                "entetes.hsts",
                "Strict-Transport-Security (HSTS)",
                "strict-transport-security",
                "max-age=",
            ),
            en_tete(
                "entetes.csp",
                "Content-Security-Policy",
                "content-security-policy",
                "default-src",
            ),
            en_tete(
                "entetes.frame",
                "CSP frame-ancestors 'none' (anti-clickjacking)",
                "content-security-policy",
                "frame-ancestors 'none'",
            ),
            en_tete(
                "entetes.nosniff",
                "X-Content-Type-Options",
                "x-content-type-options",
                "nosniff",
            ),
            en_tete("entetes.referrer", "Referrer-Policy", "referrer-policy", "same-origin"),
            en_tete(
                "entetes.cache",
                "Cache-Control : aucune page mise en cache",
                "cache-control",
                "no-store",
            ),
        ],
    };

    // 4. Session praticien.
    let attributs = attributs_cookie(app);
    let session_section = Section {
        titre: "Session praticien",
        verifications: vec![
            v(
                "session.inactivite",
                "Expiration après inactivité",
                format!("{} minutes", auth::INACTIVITE_MINUTES),
                Statut::Ok,
            ),
            v(
                "session.max",
                "Durée maximale d'une session",
                format!("{} heures", auth::DUREE_MAX_HEURES),
                Statut::Ok,
            ),
            v(
                "session.cookie",
                "Cookie de session",
                format!("{} · {attributs}", nom_cookie(app)),
                ok_si(attributs.contains("HttpOnly") && attributs.contains("SameSite=Strict")),
            ),
            v(
                "session.secure",
                "Cookie Secure (HTTPS uniquement)",
                if app.production {
                    "oui".into()
                } else {
                    "non en local (HTTP)".into()
                },
                if app.production { Statut::Ok } else { Statut::Info },
            ),
            v(
                "session.blocage",
                "Anti force brute",
                format!(
                    "{} échecs depuis une IP → blocage {} minutes",
                    auth::ECHECS_MAX,
                    auth::BLOCAGE.as_secs() / 60
                ),
                Statut::Ok,
            ),
            v(
                "session.stockage",
                "Jeton de session",
                "aléatoire 32 octets, seule son empreinte SHA-256 est en base".into(),
                Statut::Ok,
            ),
        ],
    };

    // 5. Journal d'accès : volume et trigger d'ajout seul.
    let journal: Option<(i64, bool, bool)> = sqlx::query_as(
        "SELECT (SELECT count(*) FROM journal_acces),
                EXISTS (SELECT 1 FROM pg_trigger WHERE tgrelid = 'journal_acces'::regclass
                        AND tgname = 'journal_acces_ajout_seul' AND tgenabled <> 'D'),
                EXISTS (SELECT 1 FROM pg_trigger WHERE tgrelid = 'journal_acces'::regclass
                        AND tgname = 'journal_acces_sans_truncate' AND tgenabled <> 'D')",
    )
    .fetch_optional(&app.pool)
    .await
    .ok()
    .flatten();
    let (entrees, trigger, truncate) = journal.unwrap_or((0, false, false));
    let journal_section = Section {
        titre: "Journal d'accès",
        verifications: vec![
            v(
                "journal.entrees",
                "Entrées du journal (qui, quoi, quel patient, quand, IP)",
                format!("{entrees} entrée(s)"),
                ok_si(journal.is_some()),
            ),
            v(
                "journal.ajout_seul",
                "Ajout seul : UPDATE et DELETE refusés par un trigger (pg_trigger)",
                if trigger {
                    "trigger journal_acces_ajout_seul actif".into()
                } else {
                    "trigger absent".into()
                },
                ok_si(trigger),
            ),
            v(
                "journal.truncate",
                "TRUNCATE refusé",
                if truncate {
                    "trigger journal_acces_sans_truncate actif".into()
                } else {
                    "trigger absent".into()
                },
                ok_si(truncate),
            ),
        ],
    };

    // 6. Logs : le masquage est appliqué sur un jeton fictif tiré à l'instant.
    let fictif = format!("/p/{}", auth::jeton(24));
    let masque = masquer_chemin(&fictif);
    let logs_section = Section {
        titre: "Logs sans donnée patient",
        verifications: vec![
            v(
                "logs.jeton",
                "Jetons /p/… masqués dans les logs",
                format!("GET /p/{}… journalisé « GET {masque} »", &fictif[3..11]),
                ok_si(!masque.contains(&fictif[3..])),
            ),
            v(
                "logs.contenu",
                "Contenu des logs",
                "méthode, chemin masqué, statut, durée : jamais de nom, de commentaire ni de jeton".into(),
                Statut::Ok,
            ),
        ],
    };

    let sections = vec![
        base_section,
        https_section,
        en_tetes_section,
        session_section,
        journal_section,
        logs_section,
    ];
    let mut synthese = Synthese::default();
    for s in sections.iter().flat_map(|s| &s.verifications) {
        match s.statut {
            Statut::Ok => synthese.ok += 1,
            Statut::Ko => synthese.ko += 1,
            Statut::Info => synthese.info += 1,
        }
    }
    Conformite {
        genere_le: maintenant,
        mode: if app.production {
            "production".into()
        } else {
            "local".into()
        },
        synthese,
        base,
        sections,
        plateforme: PLATEFORME.into_iter().collect(),
    }
}

pub async fn page(State(app): State<Arc<App>>, headers: HeaderMap) -> Html<String> {
    let c = calculer(&app, &headers).await;
    let praticien = auth::connecte(&app, &headers).await;
    Html(crate::html::conformite(&c, praticien))
}

pub async fn json(State(app): State<Arc<App>>, headers: HeaderMap) -> Json<Conformite> {
    Json(calculer(&app, &headers).await)
}
