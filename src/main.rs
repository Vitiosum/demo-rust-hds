//! Suivi patient : espace praticien, espace patient (lien personnel) et journal d'accès.
//! Démo Clever Cloud en zone HDS (parhds). Données fictives uniquement.

mod auth;
mod conformite;
mod html;
mod migrations;
mod pages;

use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::{Request, State};
use axum::http::{header, HeaderMap, HeaderName, HeaderValue, Method, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::Router;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions, PgSslMode};
use sqlx::PgPool;

pub struct App {
    pub pool: PgPool,
    pub production: bool,
    pub tls_base: bool,
    pub login: String,
    pub password: String,
    pub limiteur: auth::Limiteur,
}

/// En-têtes de sécurité ajoutés à TOUTES les réponses (la page /conformite lit la même constante).
pub const EN_TETES: [(&str, &str); 6] = [
    ("strict-transport-security", "max-age=31536000; includeSubDomains"),
    (
        "content-security-policy",
        "default-src 'none'; style-src 'self'; img-src 'self' data:; form-action 'self'; frame-ancestors 'none'; base-uri 'none'",
    ),
    ("x-content-type-options", "nosniff"),
    // same-origin : rien vers l'extérieur, mais Origin reste envoyé sur nos propres formulaires.
    ("referrer-policy", "same-origin"),
    ("cache-control", "no-store"),
    ("x-frame-options", "DENY"),
];

/// Erreur base de données : on ne journalise que le code SQLSTATE, jamais le contenu.
pub struct ErreurApp(pub sqlx::Error);

impl From<sqlx::Error> for ErreurApp {
    fn from(e: sqlx::Error) -> Self {
        ErreurApp(e)
    }
}

impl IntoResponse for ErreurApp {
    fn into_response(self) -> Response {
        let code = self
            .0
            .as_database_error()
            .and_then(|d| d.code().map(|c| c.into_owned()))
            .unwrap_or_else(|| "connexion".into());
        println!("Erreur base de données (SQLSTATE {code})");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Html(html::message("Erreur interne", "Réessayez dans un instant.", false)),
        )
            .into_response()
    }
}

/// Journal d'accès en base : qui, quoi, quel patient, quand, IP.
pub async fn journal(
    pool: &PgPool,
    acteur: &str,
    action: &str,
    patient_id: Option<i32>,
    ip: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO journal_acces (acteur, action, patient_id, ip) VALUES ($1, $2, $3, $4)")
        .bind(acteur)
        .bind(action)
        .bind(patient_id)
        .bind(ip)
        .execute(pool)
        .await
        .map(|_| ())
}

/// Chemin tel qu'il apparaît dans les logs : le jeton patient est masqué.
pub fn masquer_chemin(chemin: &str) -> String {
    match chemin.find("/p/") {
        Some(i) => format!("{}/p/***", &chemin[..i]),
        None => chemin.to_string(),
    }
}

/// La requête vient-elle d'une page de l'app ? Sec-Fetch-Site d'abord, sinon Origin == Host.
fn meme_origine(headers: &HeaderMap) -> bool {
    if let Some(site) = headers.get("sec-fetch-site").and_then(|v| v.to_str().ok()) {
        return site == "same-origin";
    }
    let origine = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok());
    let hote = headers.get(header::HOST).and_then(|v| v.to_str().ok());
    match (origine, hote) {
        (Some(o), Some(h)) => o
            .split_once("://")
            .is_some_and(|(_, reste)| reste.trim_end_matches('/') == h),
        _ => false,
    }
}

/// Middleware unique : HTTPS forcé, CSRF, en-têtes de sécurité, une ligne de log par requête.
async fn garde(State(app): State<Arc<App>>, req: Request, next: Next) -> Response {
    let debut = Instant::now();
    let methode = req.method().clone();
    let chemin = req.uri().path().to_string();
    let https = req.headers().get("x-forwarded-proto").and_then(|v| v.to_str().ok()) == Some("https");

    let mut reponse = if app.production && !https && chemin != "/health" {
        let hote = req
            .headers()
            .get(header::HOST)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default();
        let cible = req.uri().path_and_query().map(|p| p.as_str()).unwrap_or("/");
        (
            StatusCode::MOVED_PERMANENTLY,
            [(header::LOCATION, format!("https://{hote}{cible}"))],
        )
            .into_response()
    } else if methode != Method::GET && methode != Method::HEAD && !meme_origine(req.headers()) {
        (
            StatusCode::FORBIDDEN,
            Html(html::message("Requête refusée", "Origine non autorisée.", false)),
        )
            .into_response()
    } else {
        next.run(req).await
    };

    let h = reponse.headers_mut();
    for (nom, valeur) in EN_TETES {
        h.insert(HeaderName::from_static(nom), HeaderValue::from_static(valeur));
    }
    println!(
        "{methode} {} {} {:.1}ms",
        masquer_chemin(&chemin),
        reponse.status().as_u16(),
        debut.elapsed().as_secs_f64() * 1000.0
    );
    reponse
}

async fn sante(State(app): State<Arc<App>>) -> Response {
    match sqlx::query("SELECT 1").execute(&app.pool).await {
        Ok(_) => "ok".into_response(),
        Err(_) => (StatusCode::SERVICE_UNAVAILABLE, "base injoignable").into_response(),
    }
}

fn css(contenu: &'static str) -> Response {
    ([(header::CONTENT_TYPE, "text/css; charset=utf-8")], contenu).into_response()
}

async fn introuvable() -> Response {
    (
        StatusCode::NOT_FOUND,
        Html(html::message("Page introuvable", "Cette page n'existe pas.", false)),
    )
        .into_response()
}

fn var(nom: &str) -> Option<String> {
    std::env::var(nom).ok().filter(|v| !v.trim().is_empty())
}

async fn connecter(options: PgConnectOptions, max: u32) -> Result<PgPool, String> {
    const ESSAIS: u32 = 20;
    for essai in 1..=ESSAIS {
        match PgPoolOptions::new()
            .max_connections(max)
            .acquire_timeout(Duration::from_secs(5))
            .connect_with(options.clone())
            .await
        {
            Ok(pool) => return Ok(pool),
            Err(e) if essai < ESSAIS => {
                println!("Base injoignable (essai {essai}/{ESSAIS}) : {e}");
                tokio::time::sleep(Duration::from_secs(3)).await;
            }
            Err(e) => return Err(format!("base injoignable : {e}")),
        }
    }
    unreachable!()
}

async fn demarrer() -> Result<(), String> {
    let production = var("APP_ENV").as_deref() == Some("production");
    let _ = html::PRODUCTION.set(production);

    let login = var("PRATICIEN_LOGIN").unwrap_or_else(|| "admin".into());
    let password = var("PRATICIEN_PASSWORD").ok_or("PRATICIEN_PASSWORD manquant")?;
    if production && password.chars().count() < 12 {
        return Err("PRATICIEN_PASSWORD trop court : 12 caractères minimum avec APP_ENV=production".into());
    }

    // POSTGRESQL_ADDON_URI est injectée par l'add-on lié ; DATABASE_URL sert en local.
    let (url, source) = match var("POSTGRESQL_ADDON_URI") {
        Some(u) => (u, "POSTGRESQL_ADDON_URI"),
        None => (
            var("DATABASE_URL").ok_or("POSTGRESQL_ADDON_URI absente : liez l'add-on PostgreSQL à l'application")?,
            "DATABASE_URL",
        ),
    };
    // TLS obligatoire. Le certificat de l'add-on est autosigné : Require chiffre sans vérifier la chaîne.
    let tls_base = var("PG_SSL").as_deref() != Some("disable");
    if production && !tls_base {
        return Err("PG_SSL=disable interdit avec APP_ENV=production".into());
    }
    let options = PgConnectOptions::from_str(&url)
        .map_err(|_| format!("{source} invalide"))?
        .ssl_mode(if tls_base {
            PgSslMode::Require
        } else {
            PgSslMode::Disable
        });
    let pool_max = var("DB_POOL_MAX").and_then(|v| v.parse().ok()).unwrap_or(2);

    let debut = Instant::now();
    let pool = connecter(options, pool_max).await?;
    println!(
        "Base joignable en {:.1}s ({source}, TLS {})",
        debut.elapsed().as_secs_f64(),
        if tls_base { "require" } else { "désactivé" }
    );
    migrations::appliquer(&pool)
        .await
        .map_err(|e| format!("migrations : {e}"))?;
    if var("SEED_DEMO").as_deref() == Some("true") {
        pages::donnees_demo(&pool)
            .await
            .map_err(|e| format!("données de démo : {e}"))?;
    }

    let app = Arc::new(App {
        pool,
        production,
        tls_base,
        login,
        password,
        limiteur: auth::Limiteur::default(),
    });
    let routes = Router::new()
        .route("/", get(|| async { Redirect::to("/praticien") }))
        .route("/health", get(sante))
        .route("/cc-brand.css", get(|| async { css(html::BRAND_CSS) }))
        .route("/app.css", get(|| async { css(html::APP_CSS) }))
        .route("/connexion", get(auth::page_connexion).post(auth::connexion))
        .route("/deconnexion", post(auth::deconnexion))
        .route("/praticien", get(pages::tableau_de_bord))
        .route("/praticien/patients", post(pages::creer_patient))
        .route(
            "/praticien/patients/{id}",
            get(pages::fiche_patient).post(pages::mettre_a_jour),
        )
        .route("/p/{jeton}", get(pages::espace_patient).post(pages::saisie_patient))
        .route("/journal", get(pages::journal_acces))
        .route("/conformite", get(conformite::page))
        .route("/conformite.json", get(conformite::json))
        .fallback(introuvable)
        .layer(middleware::from_fn_with_state(app.clone(), garde))
        .with_state(app.clone());

    let port: u16 = var("PORT").and_then(|v| v.parse().ok()).unwrap_or(8080);
    let ecoute = tokio::net::TcpListener::bind(("0.0.0.0", port))
        .await
        .map_err(|e| format!("port {port} : {e}"))?;
    println!(
        "Suivi patient en écoute sur 0.0.0.0:{port} (mode {})",
        if production { "production" } else { "local" }
    );
    axum::serve(ecoute, routes.into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(arret())
        .await
        .map_err(|e| e.to_string())?;
    app.pool.close().await;
    Ok(())
}

/// Arrêt propre sur SIGTERM (redéploiement Clever Cloud) ou Ctrl+C.
async fn arret() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let sigterm = async {
        if let Ok(mut s) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            s.recv().await;
        }
    };
    #[cfg(not(unix))]
    let sigterm = std::future::pending::<()>();
    tokio::select! { _ = ctrl_c => {}, _ = sigterm => {} }
    println!("Arrêt demandé : fin des requêtes en cours.");
}

#[tokio::main]
async fn main() {
    if let Err(e) = demarrer().await {
        eprintln!("Démarrage refusé : {e}");
        std::process::exit(1);
    }
}
