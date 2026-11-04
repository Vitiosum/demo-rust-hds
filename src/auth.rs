//! Connexion praticien : session côté serveur, cookie strict, blocage après 5 échecs.

use std::collections::HashMap;
use std::fmt::Write;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::{ConnectInfo, FromRequestParts, State};
use axum::http::{header, request::Parts, HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::Form;
use serde::Deserialize;

use crate::{html, journal, App, ErreurApp};

pub const INACTIVITE_MINUTES: i32 = 15;
pub const DUREE_MAX_HEURES: i32 = 8;
pub const ECHECS_MAX: u32 = 5;
pub const BLOCAGE: Duration = Duration::from_secs(15 * 60);

/// Nom du cookie. En production, le préfixe `__Host-` impose Secure, Path=/ et aucun Domain.
pub fn nom_cookie(app: &App) -> &'static str {
    if app.production {
        "__Host-session"
    } else {
        "session"
    }
}

pub fn attributs_cookie(app: &App) -> &'static str {
    if app.production {
        "Path=/; HttpOnly; SameSite=Strict; Secure"
    } else {
        "Path=/; HttpOnly; SameSite=Strict"
    }
}

/// Jeton aléatoire du système, en hexadécimal.
pub fn jeton(octets: usize) -> String {
    let mut buf = vec![0u8; octets];
    getrandom::getrandom(&mut buf).expect("générateur aléatoire du système indisponible");
    let mut out = String::with_capacity(octets * 2);
    for b in buf {
        let _ = write!(out, "{b:02x}");
    }
    out
}

/// Comparaison en temps constant : la durée ne dépend que de la longueur du secret attendu.
pub fn egal_temps_constant(saisi: &[u8], attendu: &[u8]) -> bool {
    let mut diff = saisi.len() ^ attendu.len();
    for (i, a) in attendu.iter().enumerate() {
        let s = saisi.get(i).copied().unwrap_or(0);
        diff |= (s ^ a) as usize;
    }
    std::hint::black_box(diff) == 0
}

/// IP du client. En production, derrière les load balancers Clever Cloud, le proxy ajoute
/// l'IP réelle à DROITE de X-Forwarded-For : on prend la dernière valeur de la DERNIÈRE ligne
/// d'en-tête, jamais une valeur envoyée par le client (une ligne X-Forwarded-For forgée arrive
/// avant celle du proxy). La valeur doit être une adresse IP valide, sinon on retombe sur la
/// socket. En local, l'adresse de la socket.
pub fn ip_client(app: &App, parts: &Parts) -> String {
    if app.production {
        if let Some(ip) = ip_proxy(&parts.headers) {
            return ip.to_string();
        }
    }
    parts
        .extensions
        .get::<ConnectInfo<SocketAddr>>()
        .map(|c| c.0.ip().to_string())
        .unwrap_or_else(|| "inconnue".into())
}

/// Dernière adresse de la dernière ligne X-Forwarded-For, si c'est une IP valide.
fn ip_proxy(headers: &HeaderMap) -> Option<IpAddr> {
    headers
        .get_all("x-forwarded-for")
        .iter()
        .next_back()
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.rsplit(',').next())
        .and_then(|v| v.trim().parse().ok())
}

/// IP du client, utilisable comme extracteur.
pub struct Client(pub String);

impl FromRequestParts<Arc<App>> for Client {
    type Rejection = std::convert::Infallible;
    async fn from_request_parts(parts: &mut Parts, app: &Arc<App>) -> Result<Self, Self::Rejection> {
        Ok(Client(ip_client(app, parts)))
    }
}

fn cookie<'a>(headers: &'a HeaderMap, nom: &str) -> Option<&'a str> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|c| c.trim().split_once('='))
        .find(|(k, _)| *k == nom)
        .map(|(_, v)| v)
        .filter(|v| !v.is_empty() && v.len() <= 128)
}

/// Praticien authentifié. Chaque requête prolonge la session (inactivité),
/// dans la limite de la durée maximale depuis la connexion.
pub struct Praticien {
    pub login: String,
    pub ip: String,
}

impl FromRequestParts<Arc<App>> for Praticien {
    type Rejection = Response;
    async fn from_request_parts(parts: &mut Parts, app: &Arc<App>) -> Result<Self, Response> {
        let ip = ip_client(app, parts);
        let Some(valeur) = cookie(&parts.headers, nom_cookie(app)) else {
            return Err(Redirect::to("/connexion").into_response());
        };
        let login: Option<String> = sqlx::query_scalar(
            "UPDATE sessions SET last_seen_at = now()
             WHERE token_hash = sha256($1)
               AND last_seen_at > now() - make_interval(mins => $2)
               AND created_at > now() - make_interval(hours => $3)
             RETURNING login",
        )
        .bind(valeur.as_bytes())
        .bind(INACTIVITE_MINUTES)
        .bind(DUREE_MAX_HEURES)
        .fetch_optional(&app.pool)
        .await
        .map_err(|e| ErreurApp(e).into_response())?;
        match login {
            Some(login) => Ok(Praticien { login, ip }),
            None => Err((
                [(
                    header::SET_COOKIE,
                    format!("{}=; Max-Age=0; {}", nom_cookie(app), attributs_cookie(app)),
                )],
                Redirect::to("/connexion?expiree=1"),
            )
                .into_response()),
        }
    }
}

/// Session éventuelle, sans redirection (pages publiques).
pub async fn connecte(app: &App, headers: &HeaderMap) -> bool {
    let Some(valeur) = cookie(headers, nom_cookie(app)) else {
        return false;
    };
    sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT 1 FROM sessions WHERE token_hash = sha256($1)
           AND last_seen_at > now() - make_interval(mins => $2)
           AND created_at > now() - make_interval(hours => $3))",
    )
    .bind(valeur.as_bytes())
    .bind(INACTIVITE_MINUTES)
    .bind(DUREE_MAX_HEURES)
    .fetch_one(&app.pool)
    .await
    .unwrap_or(false)
}

/// Compteur d'échecs de connexion par IP, en mémoire (propre à chaque instance).
#[derive(Default)]
pub struct Limiteur(std::sync::Mutex<HashMap<String, Tentatives>>);

pub struct Tentatives {
    echecs: u32,
    premier: Instant,
    bloque_jusqua: Option<Instant>,
}

impl Limiteur {
    /// Durée de blocage restante pour cette IP.
    pub fn bloque(&self, ip: &str) -> Option<Duration> {
        let map = self.0.lock().unwrap_or_else(|e| e.into_inner());
        map.get(ip)?.bloque_jusqua?.checked_duration_since(Instant::now())
    }

    /// Enregistre un échec ; renvoie le nombre d'essais restants (0 = IP bloquée).
    pub fn echec(&self, ip: &str) -> u32 {
        let maintenant = Instant::now();
        let mut map = self.0.lock().unwrap_or_else(|e| e.into_inner());
        map.retain(|_, t| match t.bloque_jusqua {
            Some(fin) => fin > maintenant,
            None => maintenant.duration_since(t.premier) < BLOCAGE,
        });
        let t = map.entry(ip.to_string()).or_insert(Tentatives {
            echecs: 0,
            premier: maintenant,
            bloque_jusqua: None,
        });
        t.echecs += 1;
        if t.echecs >= ECHECS_MAX {
            t.bloque_jusqua = Some(maintenant + BLOCAGE);
            0
        } else {
            ECHECS_MAX - t.echecs
        }
    }

    pub fn reussite(&self, ip: &str) {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).remove(ip);
    }
}

pub async fn page_connexion(uri: axum::http::Uri) -> Html<String> {
    let info = (uri.query() == Some("expiree=1")).then_some("Session expirée : reconnectez-vous.");
    Html(html::connexion(None, info))
}

#[derive(Deserialize)]
pub struct FormConnexion {
    identifiant: String,
    mot_de_passe: String,
}

fn bloquee(reste: Duration) -> Response {
    let minutes = reste.as_secs().div_ceil(60).max(1);
    let msg = format!("Trop de tentatives depuis cette adresse IP. Réessayez dans {minutes} minute(s).");
    (StatusCode::TOO_MANY_REQUESTS, Html(html::connexion(Some(&msg), None))).into_response()
}

pub async fn connexion(
    State(app): State<Arc<App>>,
    Client(ip): Client,
    Form(f): Form<FormConnexion>,
) -> Result<Response, ErreurApp> {
    if let Some(reste) = app.limiteur.bloque(&ip) {
        journal(&app.pool, "inconnu", "connexion_bloquee", None, &ip).await?;
        return Ok(bloquee(reste));
    }
    // Les deux comparaisons sont toujours faites (pas de court-circuit).
    let login_ok = egal_temps_constant(f.identifiant.as_bytes(), app.login.as_bytes());
    let mdp_ok = egal_temps_constant(f.mot_de_passe.as_bytes(), app.password.as_bytes());
    if !(login_ok & mdp_ok) {
        let restants = app.limiteur.echec(&ip);
        let acteur = if login_ok {
            app.login.as_str()
        } else {
            "identifiant inconnu"
        };
        journal(&app.pool, acteur, "connexion_refusee", None, &ip).await?;
        tokio::time::sleep(Duration::from_millis(300)).await;
        if restants == 0 {
            println!(
                "Connexion : IP bloquée {} minutes après {ECHECS_MAX} échecs",
                BLOCAGE.as_secs() / 60
            );
            return Ok(bloquee(BLOCAGE));
        }
        let msg = format!("Identifiant ou mot de passe incorrect. {restants} essai(s) avant blocage de 15 minutes.");
        return Ok((StatusCode::UNAUTHORIZED, Html(html::connexion(Some(&msg), None))).into_response());
    }
    app.limiteur.reussite(&ip);
    sqlx::query(
        "DELETE FROM sessions WHERE last_seen_at < now() - make_interval(mins => $1)
            OR created_at < now() - make_interval(hours => $2)",
    )
    .bind(INACTIVITE_MINUTES)
    .bind(DUREE_MAX_HEURES)
    .execute(&app.pool)
    .await?;
    let valeur = jeton(32);
    sqlx::query("INSERT INTO sessions (token_hash, login) VALUES (sha256($1), $2)")
        .bind(valeur.as_bytes())
        .bind(&app.login)
        .execute(&app.pool)
        .await?;
    journal(&app.pool, &app.login, "connexion", None, &ip).await?;
    let cookie = format!("{}={valeur}; {}", nom_cookie(&app), attributs_cookie(&app));
    Ok(([(header::SET_COOKIE, cookie)], Redirect::to("/praticien")).into_response())
}

pub async fn deconnexion(
    State(app): State<Arc<App>>,
    Client(ip): Client,
    headers: HeaderMap,
) -> Result<Response, ErreurApp> {
    if let Some(valeur) = cookie(&headers, nom_cookie(&app)) {
        let login: Option<String> =
            sqlx::query_scalar("DELETE FROM sessions WHERE token_hash = sha256($1) RETURNING login")
                .bind(valeur.as_bytes())
                .fetch_optional(&app.pool)
                .await?;
        if let Some(login) = login {
            journal(&app.pool, &login, "deconnexion", None, &ip).await?;
        }
    }
    let efface = format!("{}=; Max-Age=0; {}", nom_cookie(&app), attributs_cookie(&app));
    Ok(([(header::SET_COOKIE, efface)], Redirect::to("/connexion")).into_response())
}

#[cfg(test)]
mod tests {
    use super::ip_proxy;
    use axum::http::{HeaderMap, HeaderValue};

    fn xff(lignes: &[&str]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for l in lignes {
            h.append("x-forwarded-for", HeaderValue::from_str(l).unwrap());
        }
        h
    }

    #[test]
    fn ligne_forgee_par_le_client_ignoree() {
        let h = xff(&["1.2.3.4", "203.0.113.7"]);
        assert_eq!(ip_proxy(&h), Some("203.0.113.7".parse().unwrap()));
    }

    #[test]
    fn valeur_ajoutee_a_droite_par_le_proxy() {
        let h = xff(&["1.2.3.4, 203.0.113.7"]);
        assert_eq!(ip_proxy(&h), Some("203.0.113.7".parse().unwrap()));
    }

    #[test]
    fn ipv6_acceptee() {
        let h = xff(&["2001:db8::1"]);
        assert_eq!(ip_proxy(&h), Some("2001:db8::1".parse().unwrap()));
    }

    #[test]
    fn valeur_qui_n_est_pas_une_ip_refusee() {
        assert_eq!(ip_proxy(&xff(&["1.2.3.4, pas-une-ip"])), None);
        assert_eq!(ip_proxy(&HeaderMap::new()), None);
    }
}
