//! Espace praticien, espace patient et journal d'accès.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::Form;
use chrono::NaiveDateTime;
use serde::Deserialize;
use sqlx::postgres::PgRow;
use sqlx::Row;

use crate::auth::{jeton, Client, Praticien};
use crate::{html, journal, App, ErreurApp};

pub struct LignePatient {
    pub id: i32,
    pub prenom: String,
    pub nom: String,
    pub motif: String,
    pub rdv: Option<NaiveDateTime>,
    pub non_lus: i64,
    pub alerte: bool,
}

pub struct Patient {
    pub id: i32,
    pub prenom: String,
    pub nom: String,
    pub motif: String,
    pub consignes: String,
    pub rdv: Option<NaiveDateTime>,
    pub token: String,
}

pub struct Suivi {
    pub cree_le: NaiveDateTime,
    pub douleur: i16,
    pub tension: Option<String>,
    pub commentaire: String,
}

pub struct EntreeJournal {
    pub quand: String,
    pub acteur: String,
    pub action: String,
    pub patient_id: Option<i32>,
    pub patient: Option<String>,
    pub ip: Option<String>,
}

// Lecture des lignes par nom de colonne (pas de derive FromRow : pas de proc-macro sqlx au build).
impl LignePatient {
    fn lire(r: PgRow) -> Result<Self, sqlx::Error> {
        Ok(Self {
            id: r.try_get("id")?,
            prenom: r.try_get("prenom")?,
            nom: r.try_get("nom")?,
            motif: r.try_get("motif")?,
            rdv: r.try_get("rdv")?,
            non_lus: r.try_get("non_lus")?,
            alerte: r.try_get("alerte")?,
        })
    }
}

impl Patient {
    fn lire(r: PgRow) -> Result<Self, sqlx::Error> {
        Ok(Self {
            id: r.try_get("id")?,
            prenom: r.try_get("prenom")?,
            nom: r.try_get("nom")?,
            motif: r.try_get("motif")?,
            consignes: r.try_get("consignes")?,
            rdv: r.try_get("rdv")?,
            token: r.try_get("token")?,
        })
    }
}

impl Suivi {
    fn lire(r: PgRow) -> Result<Self, sqlx::Error> {
        Ok(Self {
            cree_le: r.try_get("cree_le")?,
            douleur: r.try_get("douleur")?,
            tension: r.try_get("tension")?,
            commentaire: r.try_get("commentaire")?,
        })
    }
}

impl EntreeJournal {
    fn lire(r: PgRow) -> Result<Self, sqlx::Error> {
        Ok(Self {
            quand: r.try_get("quand")?,
            acteur: r.try_get("acteur")?,
            action: r.try_get("action")?,
            patient_id: r.try_get("patient_id")?,
            patient: r.try_get("patient")?,
            ip: r.try_get("ip")?,
        })
    }
}

// Les dates sont stockées en TIMESTAMPTZ et converties à l'heure de Paris par PostgreSQL.
const COLONNES_PATIENT: &str =
    "id, prenom, nom, motif, consignes, prochain_rdv AT TIME ZONE 'Europe/Paris' AS rdv, token";
const COLONNES_SUIVI: &str = "created_at AT TIME ZONE 'Europe/Paris' AS cree_le, douleur, tension, commentaire";

fn non_trouve() -> Response {
    (
        StatusCode::NOT_FOUND,
        Html(html::message(
            "Page introuvable",
            "Ce lien n'existe pas ou n'est plus valide.",
            false,
        )),
    )
        .into_response()
}

/// Champ de formulaire nettoyé et tronqué (en caractères, jamais au milieu d'un caractère).
fn champ(v: &str, max: usize) -> String {
    v.trim().chars().take(max).collect()
}

/// Valeur d'un champ datetime-local (heure de Paris).
fn date_locale(v: Option<&str>) -> Option<NaiveDateTime> {
    let v = v?.trim();
    NaiveDateTime::parse_from_str(v, "%Y-%m-%dT%H:%M")
        .or_else(|_| NaiveDateTime::parse_from_str(v, "%Y-%m-%dT%H:%M:%S"))
        .ok()
}

pub async fn tableau_de_bord(State(app): State<Arc<App>>, moi: Praticien) -> Result<Html<String>, ErreurApp> {
    let patients = sqlx::query(
        "SELECT p.id, p.prenom, p.nom, p.motif, p.prochain_rdv AT TIME ZONE 'Europe/Paris' AS rdv,
                COUNT(s.id) FILTER (WHERE NOT s.lu) AS non_lus,
                COALESCE(BOOL_OR(s.douleur >= 7 AND NOT s.lu), false) AS alerte
         FROM patients p LEFT JOIN suivis s ON s.patient_id = p.id
         GROUP BY p.id
         ORDER BY alerte DESC, p.prochain_rdv NULLS LAST, p.id",
    )
    .try_map(LignePatient::lire)
    .fetch_all(&app.pool)
    .await?;
    journal(&app.pool, &moi.login, "liste_patients", None, &moi.ip).await?;
    Ok(Html(html::tableau_de_bord(&patients)))
}

#[derive(Deserialize)]
pub struct NouveauPatient {
    prenom: String,
    nom: String,
    motif: String,
    #[serde(default)]
    consignes: String,
    prochain_rdv: Option<String>,
}

pub async fn creer_patient(
    State(app): State<Arc<App>>,
    moi: Praticien,
    Form(f): Form<NouveauPatient>,
) -> Result<Response, ErreurApp> {
    let (prenom, nom, motif) = (champ(&f.prenom, 80), champ(&f.nom, 80), champ(&f.motif, 200));
    if prenom.is_empty() || nom.is_empty() || motif.is_empty() {
        return Ok((
            StatusCode::BAD_REQUEST,
            Html(html::message(
                "Champs manquants",
                "Prénom, nom et motif sont obligatoires.",
                true,
            )),
        )
            .into_response());
    }
    let id: i32 = sqlx::query_scalar(
        "INSERT INTO patients (prenom, nom, motif, consignes, prochain_rdv, token)
         VALUES ($1, $2, $3, $4, $5::timestamp AT TIME ZONE 'Europe/Paris', $6) RETURNING id",
    )
    .bind(&prenom)
    .bind(&nom)
    .bind(&motif)
    .bind(champ(&f.consignes, 2000))
    .bind(date_locale(f.prochain_rdv.as_deref()))
    .bind(jeton(24))
    .fetch_one(&app.pool)
    .await?;
    journal(&app.pool, &moi.login, "creation_patient", Some(id), &moi.ip).await?;
    Ok(Redirect::to(&format!("/praticien/patients/{id}")).into_response())
}

/// Origine publique de l'app, déduite de la requête (domaine personnalisé compris).
fn origine(headers: &HeaderMap) -> String {
    let proto = headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("http");
    let proto = if proto == "https" { "https" } else { "http" };
    let host = headers.get("host").and_then(|v| v.to_str().ok()).unwrap_or("localhost");
    format!("{proto}://{host}")
}

pub async fn fiche_patient(
    State(app): State<Arc<App>>,
    moi: Praticien,
    Path(id): Path<i32>,
    headers: HeaderMap,
) -> Result<Response, ErreurApp> {
    let patient = sqlx::query(&format!("SELECT {COLONNES_PATIENT} FROM patients WHERE id = $1"))
        .bind(id)
        .try_map(Patient::lire)
        .fetch_optional(&app.pool)
        .await?;
    let Some(p) = patient else { return Ok(non_trouve()) };
    let suivis = sqlx::query(&format!(
        "SELECT {COLONNES_SUIVI} FROM suivis WHERE patient_id = $1 ORDER BY created_at DESC"
    ))
    .bind(id)
    .try_map(Suivi::lire)
    .fetch_all(&app.pool)
    .await?;
    sqlx::query("UPDATE suivis SET lu = true WHERE patient_id = $1 AND NOT lu")
        .bind(id)
        .execute(&app.pool)
        .await?;
    journal(&app.pool, &moi.login, "consultation_dossier", Some(id), &moi.ip).await?;
    let lien = format!("{}/p/{}", origine(&headers), p.token);
    Ok(Html(html::fiche_patient(&p, &suivis, &lien)).into_response())
}

#[derive(Deserialize)]
pub struct MiseAJour {
    #[serde(default)]
    consignes: String,
    prochain_rdv: Option<String>,
}

pub async fn mettre_a_jour(
    State(app): State<Arc<App>>,
    moi: Praticien,
    Path(id): Path<i32>,
    Form(f): Form<MiseAJour>,
) -> Result<Response, ErreurApp> {
    let n = sqlx::query(
        "UPDATE patients SET consignes = $1,
                prochain_rdv = COALESCE($2::timestamp AT TIME ZONE 'Europe/Paris', prochain_rdv)
         WHERE id = $3",
    )
    .bind(champ(&f.consignes, 2000))
    .bind(date_locale(f.prochain_rdv.as_deref()))
    .bind(id)
    .execute(&app.pool)
    .await?
    .rows_affected();
    if n == 0 {
        return Ok(non_trouve());
    }
    journal(&app.pool, &moi.login, "mise_a_jour_suivi", Some(id), &moi.ip).await?;
    Ok(Redirect::to(&format!("/praticien/patients/{id}")).into_response())
}

async fn patient_par_jeton(app: &App, jeton: &str) -> Result<Option<Patient>, sqlx::Error> {
    if jeton.len() > 128 {
        return Ok(None);
    }
    sqlx::query(&format!("SELECT {COLONNES_PATIENT} FROM patients WHERE token = $1"))
        .bind(jeton)
        .try_map(Patient::lire)
        .fetch_optional(&app.pool)
        .await
}

pub async fn espace_patient(
    State(app): State<Arc<App>>,
    Client(ip): Client,
    Path(jeton): Path<String>,
    uri: Uri,
) -> Result<Response, ErreurApp> {
    let Some(p) = patient_par_jeton(&app, &jeton).await? else {
        return Ok(non_trouve());
    };
    let suivis = sqlx::query(&format!(
        "SELECT {COLONNES_SUIVI} FROM suivis WHERE patient_id = $1 ORDER BY created_at DESC LIMIT 10"
    ))
    .bind(p.id)
    .try_map(Suivi::lire)
    .fetch_all(&app.pool)
    .await?;
    journal(&app.pool, "patient", "consultation_espace", Some(p.id), &ip).await?;
    Ok(Html(html::espace_patient(&p, &suivis, uri.query() == Some("merci=1"))).into_response())
}

#[derive(Deserialize)]
pub struct Saisie {
    douleur: String,
    #[serde(default)]
    tension: String,
    #[serde(default)]
    commentaire: String,
}

pub async fn saisie_patient(
    State(app): State<Arc<App>>,
    Client(ip): Client,
    Path(jeton): Path<String>,
    Form(f): Form<Saisie>,
) -> Result<Response, ErreurApp> {
    let Some(p) = patient_par_jeton(&app, &jeton).await? else {
        return Ok(non_trouve());
    };
    let douleur = f.douleur.trim().parse::<i16>().unwrap_or(0).clamp(0, 10);
    let tension = Some(champ(&f.tension, 20)).filter(|t| !t.is_empty());
    sqlx::query("INSERT INTO suivis (patient_id, douleur, tension, commentaire) VALUES ($1, $2, $3, $4)")
        .bind(p.id)
        .bind(douleur)
        .bind(tension)
        .bind(champ(&f.commentaire, 1000))
        .execute(&app.pool)
        .await?;
    journal(&app.pool, "patient", "saisie_suivi", Some(p.id), &ip).await?;
    Ok(Redirect::to(&format!("/p/{jeton}?merci=1")).into_response())
}

pub async fn journal_acces(State(app): State<Arc<App>>, moi: Praticien) -> Result<Html<String>, ErreurApp> {
    journal(&app.pool, &moi.login, "consultation_journal", None, &moi.ip).await?;
    let entrees = sqlx::query(
        "SELECT to_char(j.created_at AT TIME ZONE 'Europe/Paris', 'DD/MM/YYYY HH24:MI:SS') AS quand,
                j.acteur, j.action, j.patient_id, p.prenom || ' ' || p.nom AS patient, j.ip
         FROM journal_acces j LEFT JOIN patients p ON p.id = j.patient_id
         ORDER BY j.id DESC LIMIT 200",
    )
    .try_map(EntreeJournal::lire)
    .fetch_all(&app.pool)
    .await?;
    let total: i64 = sqlx::query_scalar("SELECT count(*) FROM journal_acces")
        .fetch_one(&app.pool)
        .await?;
    Ok(Html(html::journal(&entrees, total)))
}

/// Données de démonstration, entièrement fictives (mêmes patients que la version Node).
pub async fn donnees_demo(pool: &sqlx::PgPool) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    // Verrou : deux instances qui démarrent ensemble ne créent pas deux fois les patients.
    sqlx::query("SELECT pg_advisory_xact_lock(424242)")
        .execute(&mut *tx)
        .await?;
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM patients")
        .fetch_one(&mut *tx)
        .await?;
    if n > 0 {
        return tx.commit().await;
    }
    // (prénom, nom, motif, consignes, RDV dans N jours, douleur, tension)
    type Fictif = (
        &'static str,
        &'static str,
        &'static str,
        &'static str,
        i32,
        i16,
        Option<&'static str>,
    );
    let patients: [Fictif; 3] = [
        (
            "Camille",
            "Martin",
            "Suivi post-opératoire genou",
            "Glace 3 fois par jour, kiné 2 fois par semaine.",
            3,
            8,
            None,
        ),
        (
            "Lucas",
            "Bernard",
            "Hypertension, ajustement du traitement",
            "Prendre la tension matin et soir.",
            7,
            2,
            Some("14/9"),
        ),
        (
            "Inès",
            "Petit",
            "Lombalgie chronique",
            "Marche 20 minutes par jour.",
            14,
            4,
            None,
        ),
    ];
    for (prenom, nom, motif, consignes, jours, douleur, tension) in patients {
        let id: i32 = sqlx::query_scalar(
            "INSERT INTO patients (prenom, nom, motif, consignes, prochain_rdv, token)
             VALUES ($1, $2, $3, $4, now() + make_interval(days => $5), $6) RETURNING id",
        )
        .bind(prenom)
        .bind(nom)
        .bind(motif)
        .bind(consignes)
        .bind(jours)
        .bind(jeton(24))
        .fetch_one(&mut *tx)
        .await?;
        sqlx::query("INSERT INTO suivis (patient_id, douleur, tension, commentaire) VALUES ($1, $2, $3, 'Saisie de démonstration')")
            .bind(id)
            .bind(douleur)
            .bind(tension)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    println!("Données de démo créées (3 patients fictifs).");
    Ok(())
}
