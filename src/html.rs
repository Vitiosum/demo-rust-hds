//! Pages HTML rendues côté serveur, sans moteur de templates.
//! Toute donnée affichée passe par `esc()`.

use std::fmt::Write;
use std::sync::OnceLock;

use chrono::{Datelike, NaiveDateTime};

use crate::conformite::{Conformite, Statut};
use crate::pages::{EntreeJournal, LignePatient, Patient, Suivi};

pub const BRAND_CSS: &str = include_str!("../static/cc-brand.css");
pub const APP_CSS: &str = include_str!("../static/app.css");
const LOGO: &str = include_str!("../static/cc-logo.svg");
const FAVICON: &str = "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 101 116'%3E%3Cpolygon fill='%23AB2B4A' points='100.423 29.134 100.423 86.868 89.721 73.062 83.76 58.003 83.756 58.003 89.418 42.731'/%3E%3Cpolygon fill='%23E87A68' points='.429 29.134 .429 86.868 9.889 73.481 17.092 58.003 17.096 58.003 9.773 43.319'/%3E%3Cpolygon fill='%23D74D4E' points='83.756 58.003 52.241 60.003 17.092 58.003 31.76 29.134 50.426 .269 70.722 31.961'/%3E%3Cpolygon fill='%23BD3246' points='100.423 29.134 83.756 58.003 50.426 .269'/%3E%3Cpolygon fill='%23F19175' points='50.426 .269 17.092 58.003 .425 29.134'/%3E%3Cpolygon fill='%23CF3942' points='83.756 58.003 72.365 82.931 50.426 115.737 27.849 81.83 17.092 58.003'/%3E%3Cpolygon fill='%23E0625B' points='50.426 115.737 .425 86.868 17.092 58.003'/%3E%3Cpolygon fill='%239A244F' points='50.422 115.737 100.423 86.868 83.756 58.003'/%3E%3C/svg%3E";

/// Mode production, fixé une fois au démarrage (bandeau des pages).
pub static PRODUCTION: OnceLock<bool> = OnceLock::new();

fn production() -> bool {
    *PRODUCTION.get().unwrap_or(&false)
}

pub fn esc(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    for c in v.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

const MOIS: [&str; 12] = [
    "janv.", "févr.", "mars", "avr.", "mai", "juin", "juil.", "août", "sept.", "oct.", "nov.", "déc.",
];

/// Date et heure de Paris (la conversion de fuseau est faite par PostgreSQL).
pub fn date(d: Option<NaiveDateTime>) -> String {
    match d {
        Some(d) => format!(
            "{} {} {} à {}",
            d.day(),
            MOIS[d.month0() as usize],
            d.year(),
            d.format("%H:%M")
        ),
        None => "à planifier".into(),
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum Espace {
    Public,
    Connexion,
    Patients,
    Journal,
    Conformite,
    Patient,
}

fn lien(href: &str, texte: &str, actif: bool) -> String {
    let classe = if actif {
        "cc-topbar__link is-active"
    } else {
        "cc-topbar__link"
    };
    format!(r#"<a class="{classe}" href="{href}">{texte}</a>"#)
}

pub fn page(titre: &str, espace: Espace, praticien: bool, corps: &str) -> String {
    let nav = match espace {
        Espace::Patient => String::new(),
        _ if praticien => format!(
            r#"{}{}{}<form method="post" action="/deconnexion"><button class="cc-btn cc-btn--sm" type="submit">Se déconnecter</button></form>"#,
            lien("/praticien", "Patients", espace == Espace::Patients),
            lien("/journal", "Journal d'accès", espace == Espace::Journal),
            lien("/conformite", "Conformité", espace == Espace::Conformite),
        ),
        _ => format!(
            "{}{}",
            lien("/conformite", "Conformité", espace == Espace::Conformite),
            lien("/connexion", "Espace praticien", espace == Espace::Connexion),
        ),
    };
    let bandeau = if production() {
        r#"<div class="bandeau bandeau--prod"><div class="cc-container"><span class="cc-dot cc-dot--ok"></span>Hébergé en zone HDS · Clever Cloud</div></div>"#
    } else {
        r#"<div class="bandeau bandeau--local"><div class="cc-container"><span class="cc-dot cc-dot--muted"></span>Version locale · données fictives</div></div>"#
    };
    let accueil = if espace == Espace::Patient { "#" } else { "/" };
    format!(
        r##"<!doctype html>
<html lang="fr">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="theme-color" content="#13172e">
<meta name="robots" content="noindex, nofollow">
<link rel="stylesheet" href="/cc-brand.css">
<link rel="stylesheet" href="/app.css">
<link rel="icon" href="{FAVICON}">
<title>{titre} · Suivi patient</title>
</head>
<body>
<header class="cc-topbar">
  <div class="cc-container cc-topbar__inner">
    <a href="{accueil}" aria-label="Suivi patient">{LOGO}</a>
    <span class="cc-topbar__sep"></span>
    <div class="cc-topbar__demo"><small>Démo HDS · Rust</small>Suivi patient</div>
    <span class="cc-topbar__spacer"></span>
    <nav class="cc-topbar__nav" aria-label="Navigation">{nav}</nav>
  </div>
</header>
{bandeau}
<main class="cc-container page">
{corps}
</main>
<footer class="cc-footer">
  <div class="cc-container">
    <div class="cc-footer__bottom"><span>Données fictives uniquement · démo avant-vente Clever Cloud</span><span><a href="/conformite">Bonnes pratiques HDS de cette app</a></span></div>
  </div>
</footer>
</body>
</html>"##,
        titre = esc(titre),
    )
}

pub fn message(titre: &str, texte: &str, praticien: bool) -> String {
    page(
        titre,
        Espace::Public,
        praticien,
        &format!(
            r#"<section class="cc-card cc-card--pad"><h1 class="titre">{}</h1><p class="sous-titre">{}</p></section>"#,
            esc(titre),
            esc(texte)
        ),
    )
}

pub fn connexion(erreur: Option<&str>, info: Option<&str>) -> String {
    let erreur = erreur
        .map(|e| format!(r#"<div class="cc-error" role="alert">{}</div>"#, esc(e)))
        .unwrap_or_default();
    let info = info
        .map(|i| format!(r#"<p class="note">{}</p>"#, esc(i)))
        .unwrap_or_default();
    page(
        "Connexion",
        Espace::Connexion,
        false,
        &format!(
            r#"<section class="cc-card cc-card--pad cc-card--accent connexion">
  <div class="cc-card__label">Espace praticien</div>
  <h1 class="titre">Connexion</h1>
  {info}{erreur}
  <form class="form" method="post" action="/connexion">
    <div class="cc-field"><label class="cc-label" for="identifiant">Identifiant</label>
      <input class="cc-input" id="identifiant" name="identifiant" autocomplete="username" required maxlength="100"></div>
    <div class="cc-field"><label class="cc-label" for="mot_de_passe">Mot de passe</label>
      <input class="cc-input" id="mot_de_passe" name="mot_de_passe" type="password" autocomplete="current-password" required maxlength="200"></div>
    <button class="cc-btn cc-btn--gradient" type="submit">Se connecter</button>
  </form>
  <p class="note">Session de 15 minutes d'inactivité, 8 heures au plus. 5 échecs bloquent l'adresse IP 15 minutes.</p>
</section>"#
        ),
    )
}

pub fn tableau_de_bord(patients: &[LignePatient]) -> String {
    let alertes = patients.iter().filter(|p| p.alerte).count();
    let mut lignes = String::new();
    for p in patients {
        let suivi = if p.non_lus > 0 {
            format!(
                r#"<span class="cc-badge cc-badge--info">{} nouvelle(s)</span>"#,
                p.non_lus
            )
        } else {
            r#"<span class="cc-badge cc-badge--neutral">à jour</span>"#.into()
        };
        let alerte = if p.alerte {
            r#" <span class="cc-badge cc-badge--danger">douleur ≥ 7</span>"#
        } else {
            ""
        };
        let _ = write!(
            lignes,
            r#"<tr{classe}><td><a class="lien" href="/praticien/patients/{id}">{prenom} {nom}</a><div class="meta">{motif}</div></td><td>{rdv}</td><td>{suivi}{alerte}</td></tr>"#,
            classe = if p.alerte { r#" class="ligne-alerte""# } else { "" },
            id = p.id,
            prenom = esc(&p.prenom),
            nom = esc(&p.nom),
            motif = esc(&p.motif),
            rdv = date(p.rdv),
        );
    }
    if lignes.is_empty() {
        lignes = r#"<tr><td colspan="3" class="meta">Aucun patient.</td></tr>"#.into();
    }
    let bandeau_alerte = if alertes > 0 {
        format!(
            r#"<div class="alerte" role="alert"><strong>{alertes} alerte(s) :</strong> douleur de 7/10 ou plus signalée, saisie non lue.</div>"#
        )
    } else {
        String::new()
    };
    page(
        "Mes patients",
        Espace::Patients,
        true,
        &format!(
            r#"<div><h1 class="titre">Mes patients</h1><p class="sous-titre">{n} patient(s) suivi(s) · prochains rendez-vous</p></div>
{bandeau_alerte}
<section class="cc-card">
  <div class="table-wrap"><table class="cc-table">
    <thead><tr><th>Patient</th><th>Prochain RDV</th><th>Suivi</th></tr></thead>
    <tbody>{lignes}</tbody>
  </table></div>
</section>
<section class="cc-card cc-card--pad">
  <h2 class="cc-card__title">Nouveau patient</h2>
  <form class="form cc-mt-2" method="post" action="/praticien/patients">
    <div class="form-2">
      <div class="cc-field"><label class="cc-label" for="prenom">Prénom</label><input class="cc-input" id="prenom" name="prenom" required maxlength="80"></div>
      <div class="cc-field"><label class="cc-label" for="nom">Nom</label><input class="cc-input" id="nom" name="nom" required maxlength="80"></div>
    </div>
    <div class="cc-field"><label class="cc-label" for="motif">Motif du suivi</label><input class="cc-input" id="motif" name="motif" required maxlength="200"></div>
    <div class="cc-field"><label class="cc-label" for="consignes">Consignes pour le patient</label><textarea class="cc-textarea" id="consignes" name="consignes" maxlength="2000"></textarea></div>
    <div class="cc-field"><label class="cc-label" for="prochain_rdv">Prochain rendez-vous</label><input class="cc-input" id="prochain_rdv" type="datetime-local" name="prochain_rdv"></div>
    <div><button class="cc-btn cc-btn--gradient" type="submit">Créer et générer le lien patient</button></div>
  </form>
</section>"#,
            n = patients.len(),
        ),
    )
}

pub fn fiche_patient(p: &Patient, suivis: &[Suivi], lien_patient: &str) -> String {
    let mut lignes = String::new();
    for s in suivis {
        let _ = write!(
            lignes,
            r#"<tr><td>{date}</td><td class="{classe}">{douleur}/10</td><td>{tension}</td><td>{commentaire}</td></tr>"#,
            date = date(Some(s.cree_le)),
            classe = if s.douleur >= 7 { "douleur-haute" } else { "" },
            douleur = s.douleur,
            tension = s.tension.as_deref().map(esc).unwrap_or_else(|| "-".into()),
            commentaire = esc(&s.commentaire),
        );
    }
    if lignes.is_empty() {
        lignes = r#"<tr><td colspan="4" class="meta">Pas encore de saisie.</td></tr>"#.into();
    }
    let rdv_valeur = p
        .rdv
        .map(|d| d.format("%Y-%m-%dT%H:%M").to_string())
        .unwrap_or_default();
    let nom = format!("{} {}", p.prenom, p.nom);
    page(
        &nom,
        Espace::Patients,
        true,
        &format!(
            r#"<div><a class="retour" href="/praticien">← Mes patients</a><h1 class="titre">{nom}</h1><p class="sous-titre">{motif} · prochain RDV {rdv}</p></div>
<section class="cc-card cc-card--pad cc-card--accent">
  <div class="cc-card__label">Lien personnel du patient</div>
  <p class="cc-mt-1"><a class="lien-patient" href="{lien}">{lien}</a></p>
  <p class="note">À transmettre au patient par un canal sûr. Le jeton est aléatoire (24 octets) et n'apparaît jamais dans les logs.</p>
</section>
<section class="cc-card">
  <h2 class="cc-card__title">Saisies du patient</h2>
  <div class="table-wrap cc-mt-1"><table class="cc-table">
    <thead><tr><th>Date</th><th>Douleur</th><th>Tension</th><th>Commentaire</th></tr></thead>
    <tbody>{lignes}</tbody>
  </table></div>
</section>
<section class="cc-card cc-card--pad">
  <h2 class="cc-card__title">Mettre à jour le suivi</h2>
  <form class="form cc-mt-2" method="post" action="/praticien/patients/{id}">
    <div class="cc-field"><label class="cc-label" for="consignes">Consignes</label><textarea class="cc-textarea" id="consignes" name="consignes" maxlength="2000">{consignes}</textarea></div>
    <div class="cc-field"><label class="cc-label" for="prochain_rdv">Prochain rendez-vous</label><input class="cc-input" id="prochain_rdv" type="datetime-local" name="prochain_rdv" value="{rdv_valeur}"></div>
    <div><button class="cc-btn cc-btn--gradient" type="submit">Enregistrer</button></div>
  </form>
</section>"#,
            nom = esc(&nom),
            motif = esc(&p.motif),
            rdv = date(p.rdv),
            lien = esc(lien_patient),
            id = p.id,
            consignes = esc(&p.consignes),
        ),
    )
}

pub fn espace_patient(p: &Patient, suivis: &[Suivi], merci: bool) -> String {
    let mut lignes = String::new();
    for s in suivis {
        let _ = write!(
            lignes,
            "<tr><td>{}</td><td>{}/10</td><td>{}</td></tr>",
            date(Some(s.cree_le)),
            s.douleur,
            esc(&s.commentaire)
        );
    }
    if lignes.is_empty() {
        lignes = r#"<tr><td class="meta">Aucune saisie.</td></tr>"#.into();
    }
    let consignes = if p.consignes.trim().is_empty() {
        r#"<span class="meta">Aucune pour le moment.</span>"#.to_string()
    } else {
        esc(&p.consignes).replace('\n', "<br>")
    };
    let merci = if merci {
        r#"<div class="succes" role="status">Merci, votre médecin a bien reçu votre saisie.</div>"#
    } else {
        ""
    };
    let options: String = (0..=10).map(|n| format!("<option>{n}</option>")).collect();
    page(
        "Mon suivi",
        Espace::Patient,
        false,
        &format!(
            r#"{merci}
<div><h1 class="titre">Bonjour {prenom}</h1><p class="sous-titre">Votre espace de suivi personnel</p></div>
<div class="cc-grid cc-grid--2">
  <section class="cc-card cc-card--pad cc-card--accent">
    <div class="cc-card__label">Prochain rendez-vous</div>
    <div class="cc-card__title cc-mt-1">{rdv}</div>
    <div class="cc-card__label cc-mt-3">Consignes de votre médecin</div>
    <p class="cc-mt-1">{consignes}</p>
  </section>
  <section class="cc-card cc-card--pad">
    <h2 class="cc-card__title">Comment allez-vous aujourd'hui ?</h2>
    <form class="form cc-mt-2" method="post">
      <div class="cc-field"><label class="cc-label" for="douleur">Niveau de douleur (0 à 10)</label><select class="cc-select" id="douleur" name="douleur">{options}</select></div>
      <div class="cc-field"><label class="cc-label" for="tension">Tension (facultatif, ex. 12/8)</label><input class="cc-input" id="tension" name="tension" maxlength="20"></div>
      <div class="cc-field"><label class="cc-label" for="commentaire">Un mot pour votre médecin</label><textarea class="cc-textarea" id="commentaire" name="commentaire" maxlength="1000"></textarea></div>
      <div><button class="cc-btn cc-btn--gradient" type="submit">Envoyer</button></div>
    </form>
    <p class="note">En cas d'urgence, appelez le 15.</p>
  </section>
</div>
<section class="cc-card">
  <h2 class="cc-card__title">Mes dernières saisies</h2>
  <div class="table-wrap cc-mt-1"><table class="cc-table"><tbody>{lignes}</tbody></table></div>
</section>"#,
            prenom = esc(&p.prenom),
            rdv = date(p.rdv),
        ),
    )
}

pub fn journal(entrees: &[EntreeJournal], total: i64) -> String {
    let mut lignes = String::new();
    for e in entrees {
        let patient = match (e.patient_id, &e.patient) {
            (Some(id), Some(nom)) => format!(r#"<a class="lien" href="/praticien/patients/{id}">{}</a>"#, esc(nom)),
            (Some(id), None) => format!("#{id}"),
            _ => "-".into(),
        };
        let _ = write!(
            lignes,
            r#"<tr><td class="mono">{date}</td><td>{acteur}</td><td><span class="cc-badge cc-badge--neutral">{action}</span></td><td>{patient}</td><td class="mono">{ip}</td></tr>"#,
            date = e.quand,
            acteur = esc(&e.acteur),
            action = esc(&e.action),
            ip = esc(e.ip.as_deref().unwrap_or("-")),
        );
    }
    if lignes.is_empty() {
        lignes = r#"<tr><td colspan="5" class="meta">Journal vide.</td></tr>"#.into();
    }
    page(
        "Journal d'accès",
        Espace::Journal,
        true,
        &format!(
            r#"<div><h1 class="titre">Journal d'accès</h1><p class="sous-titre">{total} entrée(s) · 200 dernières affichées · table en ajout seul (trigger PostgreSQL)</p></div>
<section class="cc-card">
  <div class="table-wrap"><table class="cc-table">
    <thead><tr><th>Quand (Paris)</th><th>Qui</th><th>Quoi</th><th>Patient</th><th>IP</th></tr></thead>
    <tbody>{lignes}</tbody>
  </table></div>
</section>"#
        ),
    )
}

fn icone(s: Statut) -> &'static str {
    match s {
        Statut::Ok => r#"<span class="check__icone check__icone--ok" aria-label="conforme">✅</span>"#,
        Statut::Ko => r#"<span class="check__icone check__icone--ko" aria-label="non conforme">❌</span>"#,
        Statut::Info => r#"<span class="check__icone" aria-label="information">ℹ️</span>"#,
    }
}

pub fn conformite(c: &Conformite, praticien: bool) -> String {
    let mut sections = String::new();
    for s in &c.sections {
        let mut items = String::new();
        for v in &s.verifications {
            let _ = write!(
                items,
                r#"<li class="check">{icone}<div><div class="check__titre">{libelle}</div><div class="check__valeur">{valeur}</div></div></li>"#,
                icone = icone(v.statut),
                libelle = esc(&v.libelle),
                valeur = esc(&v.valeur),
            );
        }
        let _ = write!(
            sections,
            r#"<section class="cc-card cc-card--pad"><h2 class="cc-card__title">{}</h2><ul class="checks">{items}</ul></section>"#,
            esc(s.titre)
        );
    }
    let mut plateforme = String::new();
    for p in &c.plateforme {
        let _ = write!(
            plateforme,
            r#"<li class="check"><span class="check__icone" aria-hidden="true">☁️</span><div><div class="check__titre">{}</div><div class="check__valeur">{}</div></div></li>"#,
            esc(p.point),
            esc(p.verification)
        );
    }
    let synthese = if c.synthese.ko == 0 {
        format!(
            r#"<span class="cc-pill cc-pill--ok"><span class="cc-dot"></span>{} vérifications OK</span>"#,
            c.synthese.ok
        )
    } else {
        format!(
            r#"<span class="cc-badge cc-badge--danger">{} point(s) non conforme(s)</span>"#,
            c.synthese.ko
        )
    };
    page(
        "Conformité HDS",
        Espace::Conformite,
        praticien,
        &format!(
            r#"<div>
  <div class="cc-eyebrow">Vérifié en direct · {genere}</div>
  <h1 class="titre">Bonnes pratiques HDS</h1>
  <p class="sous-titre">Page publique, sans aucune donnée patient. Chaque ligne est calculée à l'instant, à partir de cette requête et de la base. Version JSON : <a class="lien" href="/conformite.json">/conformite.json</a></p>
  <div class="cc-flex cc-mt-2">{synthese}<span class="cc-pill cc-pill--mono">mode {mode}</span></div>
</div>
<div class="cc-grid cc-grid--2">{sections}</div>
<section class="cc-platform">
  <div class="cc-platform__head"><h3>Côté plateforme Clever Cloud</h3><span class="cc-pill cc-pill--mono">scripts/verif-hds.sh</span></div>
  <ul class="checks checks--pad">{plateforme}</ul>
  <div class="cc-platform__foot">Ces points ne se lisent pas depuis l'application : le script les vérifie avec clever-tools et l'API Clever Cloud, sans afficher aucun secret.</div>
</section>"#,
            genere = esc(&c.genere_le),
            mode = esc(&c.mode),
        ),
    )
}
