//! Pages HTML rendues côté serveur, sans moteur de templates.
//! Toute donnée affichée passe par `esc()`.
//!
//! Univers : le dossier médical papier. L'espace praticien est une chemise cartonnée à onglets,
//! chaque patient une fiche, le lien patient une carte de rendez-vous, le journal un registre
//! numéroté et /conformite un rapport d'inspection tamponné. Aucune ressource tierce : polices
//! système, dessins en SVG, et la CSP interdit tout script.

use std::fmt::Write;
use std::sync::OnceLock;

use chrono::{Datelike, NaiveDateTime};

use crate::conformite::{Conformite, Statut};
use crate::pages::{EntreeJournal, LignePatient, Patient, Suivi};
use crate::Inscription;

pub const BRAND_CSS: &str = include_str!("../static/cc-brand.css");
pub const APP_CSS: &str = include_str!("../static/app.css");
const LOGO: &str = include_str!("../static/cc-logo.svg");
const BADGE: &str = include_str!("../static/cc-badge.svg");
const FAVICON: &str = "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 101 116'%3E%3Cpolygon fill='%23AB2B4A' points='100.423 29.134 100.423 86.868 89.721 73.062 83.76 58.003 83.756 58.003 89.418 42.731'/%3E%3Cpolygon fill='%23E87A68' points='.429 29.134 .429 86.868 9.889 73.481 17.092 58.003 17.096 58.003 9.773 43.319'/%3E%3Cpolygon fill='%23D74D4E' points='83.756 58.003 52.241 60.003 17.092 58.003 31.76 29.134 50.426 .269 70.722 31.961'/%3E%3Cpolygon fill='%23BD3246' points='100.423 29.134 83.756 58.003 50.426 .269'/%3E%3Cpolygon fill='%23F19175' points='50.426 .269 17.092 58.003 .425 29.134'/%3E%3Cpolygon fill='%23CF3942' points='83.756 58.003 72.365 82.931 50.426 115.737 27.849 81.83 17.092 58.003'/%3E%3Cpolygon fill='%23E0625B' points='50.426 115.737 .425 86.868 17.092 58.003'/%3E%3Cpolygon fill='%239A244F' points='50.422 115.737 100.423 86.868 83.756 58.003'/%3E%3C/svg%3E";

/// Mode production, fixé une fois au démarrage (étiquette des pages).
pub static PRODUCTION: OnceLock<bool> = OnceLock::new();
/// Variables de la plateforme Clever Cloud lues au démarrage (vide en local).
pub static HEBERGEMENT: OnceLock<Vec<(&'static str, String)>> = OnceLock::new();

/// Seuil d'alerte douleur, le même que dans la requête du tableau de bord.
const SEUIL: i16 = 7;

/// Filtre SVG « encre de tampon » : grain et léger bavé, appliqué par CSS aux tampons.
const ENCRE: &str = r#"<svg class="defs" aria-hidden="true" focusable="false"><filter id="encre" x="-10%" y="-10%" width="120%" height="120%"><feTurbulence type="fractalNoise" baseFrequency="0.9" numOctaves="2" seed="7" result="grain"/><feColorMatrix in="grain" type="matrix" values="0 0 0 0 0  0 0 0 0 0  0 0 0 0 0  -2.2 0 0 0 1.95" result="trous"/><feComposite in="SourceGraphic" in2="trous" operator="in" result="tache"/><feTurbulence type="fractalNoise" baseFrequency="0.045" numOctaves="1" seed="3" result="ondes"/><feDisplacementMap in="tache" in2="ondes" scale="2.4" xChannelSelector="R" yChannelSelector="G"/></filter></svg>"#;

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

/// Date courte pour l'axe de la frise : « 4 nov. ».
fn date_courte(d: NaiveDateTime) -> String {
    format!("{} {}", d.day(), MOIS[d.month0() as usize])
}

/// Numéro de registre avec espace fine insécable : « 1 042 ».
fn numero(n: i64) -> String {
    let chiffres = n.to_string();
    let mut out = String::new();
    for (i, c) in chiffres.chars().enumerate() {
        if i > 0 && (chiffres.len() - i) % 3 == 0 {
            out.push('\u{202F}');
        }
        out.push(c);
    }
    out
}

fn pluriel(n: impl Into<i64>, singulier: &str, pluriel: &str) -> String {
    let n = n.into();
    format!("{n} {}", if n > 1 { pluriel } else { singulier })
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

/// Onglet de la chemise : la navigation est l'intercalaire du dossier.
fn onglet(href: &str, texte: &str, couleur: &str, actif: bool) -> String {
    let (classe, courant) = if actif {
        (" onglet--actif", r#" aria-current="page""#)
    } else {
        ("", "")
    };
    format!(r#"<a class="onglet onglet--{couleur}{classe}" href="{href}"{courant}>{texte}</a>"#)
}

/// Panneau « Running on Clever Cloud », en étiquette d'archivage collée sous la chemise.
fn hebergement() -> String {
    let vars = HEBERGEMENT.get().map(Vec::as_slice).unwrap_or(&[]);
    let contenu = if vars.is_empty() {
        r#"<p class="archive__local">Local · hors plateforme</p>"#.to_string()
    } else {
        let mut kv = String::new();
        for (nom, valeur) in vars {
            let _ = write!(kv, "<div><dt>{nom}</dt><dd>{}</dd></div>", esc(valeur));
        }
        format!(r#"<dl class="archive__kv">{kv}</dl>"#)
    };
    format!(
        r#"<section class="archive" aria-labelledby="archive-titre"><h2 class="archive__titre" id="archive-titre">Running on Clever Cloud</h2>{contenu}</section>"#
    )
}

/// Badge Clever Cloud Academy, avec sa propre classe (le kit l'anime ; ici il reste immobile).
fn badge(classe: &str) -> String {
    BADGE.replacen("cc-cert__badge", classe, 1)
}

const ACADEMIE_TEXTE: &str = "Cette app tourne sur Clever Cloud. Vous aussi, certifiez-vous.";

pub fn page(titre: &str, espace: Espace, praticien: bool, corps: &str) -> String {
    let onglets = match espace {
        Espace::Patient => String::new(),
        _ if praticien => [
            onglet("/praticien", "Patients", "bleu", espace == Espace::Patients),
            onglet("/journal", "Journal d'accès", "vert", espace == Espace::Journal),
            onglet("/conformite", "Conformité", "rose", espace == Espace::Conformite),
        ]
        .concat(),
        _ => [
            onglet("/conformite", "Conformité", "rose", espace == Espace::Conformite),
            onglet("/connexion", "Espace praticien", "jaune", espace == Espace::Connexion),
        ]
        .concat(),
    };
    let onglets = if onglets.is_empty() {
        String::new()
    } else {
        format!(r#"<nav class="onglets" aria-label="Navigation">{onglets}</nav>"#)
    };
    let session = if praticien && espace != Espace::Patient {
        r#"<span class="session">Session praticien</span><form method="post" action="/deconnexion"><button class="cc-btn cc-btn--sm" type="submit">Se déconnecter</button></form>"#
    } else {
        ""
    };
    let etiquette = if production() {
        r#"<p class="etiquette etiquette--hds">Hébergé en zone HDS · Clever Cloud</p>"#
    } else {
        r#"<p class="etiquette etiquette--local">Version locale · données fictives</p>"#
    };
    // Sur /conformite, la certification est le sceau du rapport : pas de doublon en pied de page.
    let academie = if espace == Espace::Conformite {
        String::new()
    } else {
        format!(
            r#"<a class="academie" href="https://academy.clever.cloud">{}<span class="academie__texte">{ACADEMIE_TEXTE}</span><span class="academie__lien">academy.clever.cloud</span></a>"#,
            badge("academie__badge")
        )
    };
    let (classe_corps, classe_main) = match espace {
        Espace::Patient => ("espace-patient", "carnet"),
        Espace::Connexion => ("espace-connexion", "chemise chemise--fermee"),
        Espace::Conformite => ("espace-conformite", "chemise chemise--rapport"),
        _ => ("espace-praticien", "chemise"),
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
<body class="{classe_corps}">
{ENCRE}
<header class="cc-topbar">
  <div class="cc-container cc-topbar__inner">
    <a href="{accueil}" aria-label="Suivi patient">{LOGO}</a>
    <span class="cc-topbar__sep"></span>
    <div class="cc-topbar__demo"><small>Démo HDS · Rust</small>Suivi patient</div>
    <span class="cc-topbar__spacer"></span>
    <div class="cc-topbar__nav">{session}</div>
  </div>
</header>
<div class="bureau">
  <div class="cc-container">
    <div class="chemise-tete">{onglets}{etiquette}</div>
    <main class="{classe_main}">
{corps}
    </main>
    <footer class="pied">
      <div class="pied__grille">{hebergement}{academie}</div>
      <p class="pied__bas"><span>Données fictives uniquement · démo avant-vente Clever Cloud</span><a href="/conformite">Bonnes pratiques HDS de cette app</a></p>
    </footer>
  </div>
</div>
</body>
</html>"##,
        titre = esc(titre),
        hebergement = hebergement(),
    )
}

pub fn message(titre: &str, texte: &str, praticien: bool) -> String {
    page(
        titre,
        Espace::Public,
        praticien,
        &format!(
            r#"<section class="feuille feuille--message"><h1 class="titre">{}</h1><p class="sous-titre">{}</p></section>"#,
            esc(titre),
            esc(texte)
        ),
    )
}

/// Tampon encreur : texte court, cadre double, légèrement de travers.
fn tampon(texte: &str, couleur: &str) -> String {
    format!(r#"<span class="tampon tampon--{couleur}">{}</span>"#, esc(texte))
}

/// Tampon bleu du registre : la ligne que cette page vient d'inscrire au journal d'accès.
fn registre(i: &Inscription, lien: bool) -> String {
    let contenu = format!(
        r#"<span class="registre__ligne">Registre des accès</span><span class="registre__numero">n° {}</span><span class="registre__ligne">inscrit à {}</span>"#,
        numero(i.numero),
        esc(&i.heure)
    );
    if lien {
        format!(
            r#"<a class="registre" href="/journal" title="Cette consultation vient d'être inscrite au journal d'accès">{contenu}</a>"#
        )
    } else {
        format!(r#"<p class="registre">{contenu}</p>"#)
    }
}

/// Format de la frise : large (bureau), étroite (téléphone), compacte (fiche du tableau de bord).
#[derive(Clone, Copy, PartialEq)]
pub enum Format {
    Large,
    Etroite,
    Compacte,
}

/// Frise de la douleur dessinée en SVG : une saisie par point, de la plus ancienne à la plus
/// récente, zone d'alerte au-dessus du seuil. `points` : (date courte, douleur 0 à 10).
pub fn frise(points: &[(String, i16)], format: Format) -> String {
    let compacte = format == Format::Compacte;
    if points.is_empty() {
        // Dans une fiche du tableau de bord, la frise est dans un lien : un <span>, pas un <p>.
        let balise = if compacte { "span" } else { "p" };
        return format!(
            r#"<{balise} class="frise-vide">Pas encore de saisie : la frise se dessine à la première.</{balise}>"#
        );
    }
    let (l, h, g, d, haut, bas, classe, dates_toutes) = match format {
        Format::Compacte => (240.0, 64.0, 6.0, 6.0, 8.0, 8.0, " frise--compacte", 0),
        Format::Large => (640.0, 220.0, 34.0, 58.0, 28.0, 36.0, " frise--large", 8),
        Format::Etroite => (360.0, 240.0, 26.0, 46.0, 28.0, 36.0, " frise--etroite", 4),
    };
    let largeur = l - g - d;
    let hauteur = h - haut - bas;
    let y = |v: f64| haut + hauteur * (10.0 - v) / 10.0;
    let marge = if compacte { 8.0 } else { 18.0 };
    let n = points.len();
    let x = |i: usize| {
        if n == 1 {
            g + largeur / 2.0
        } else {
            g + marge + (largeur - 2.0 * marge) * i as f64 / (n - 1) as f64
        }
    };
    let mut s = String::new();
    let resume: Vec<String> = points.iter().map(|(date, v)| format!("{v}/10 le {date}")).collect();
    let _ = write!(
        s,
        r#"<svg class="frise{classe}" viewBox="0 0 {l} {h}" role="img" aria-label="Frise de la douleur : {resume}">"#,
        resume = esc(&resume.join(", ")),
    );
    // Zone d'alerte (7 à 10) et réglure du papier, une ligne par point de douleur.
    let _ = write!(
        s,
        r#"<rect class="frise__zone" x="{g}" y="{:.1}" width="{largeur}" height="{:.1}"/>"#,
        y(10.0),
        y(SEUIL as f64) - y(10.0)
    );
    if !compacte {
        for v in 0..=10 {
            let forte = if v % 5 == 0 { " frise__regle--forte" } else { "" };
            let _ = write!(
                s,
                r#"<line class="frise__regle{forte}" x1="{g}" x2="{}" y1="{yv:.1}" y2="{yv:.1}"/>"#,
                g + largeur,
                yv = y(v as f64)
            );
            if v % 5 == 0 {
                let _ = write!(
                    s,
                    r#"<text class="frise__echelle" x="{}" y="{:.1}">{v}</text>"#,
                    g - 8.0,
                    y(v as f64) + 4.0
                );
            }
        }
    }
    let ys = y(SEUIL as f64);
    let _ = write!(
        s,
        r#"<line class="frise__seuil" x1="{g}" x2="{}" y1="{ys:.1}" y2="{ys:.1}"/>"#,
        g + largeur
    );
    if !compacte {
        let _ = write!(
            s,
            r#"<text class="frise__seuil-texte" x="{}" y="{:.1}">seuil {SEUIL}</text>"#,
            g + largeur + 6.0,
            ys + 4.0
        );
    }
    // Le trait du stylo, puis les points ; un point d'alerte est entouré à l'encre rouge.
    let trace: Vec<String> = points
        .iter()
        .enumerate()
        .map(|(i, (_, v))| format!("{:.1},{:.1}", x(i), y(*v as f64)))
        .collect();
    let _ = write!(s, r#"<polyline class="frise__trait" points="{}"/>"#, trace.join(" "));
    let rayon = if compacte { 3.2 } else { 4.6 };
    for (i, (date, v)) in points.iter().enumerate() {
        let (px, py) = (x(i), y(*v as f64));
        let alerte = *v >= SEUIL;
        let _ = write!(
            s,
            r#"<circle class="frise__point{}" cx="{px:.1}" cy="{py:.1}" r="{rayon}"/>"#,
            if alerte { " frise__point--alerte" } else { "" }
        );
        if alerte {
            let k = if compacte { 0.6 } else { 1.0 };
            let _ = write!(
                s,
                r#"<path class="frise__cercle" transform="translate({px:.1} {py:.1}) scale({k})" d="M-11.5,-1 c0,-10 21,-12 23,-1 c2,11 -18,15 -23,6 c-2,-4 1,-9 6,-11"/>"#
            );
        }
        if !compacte {
            let _ = write!(
                s,
                r#"<text class="frise__valeur{}" x="{px:.1}" y="{:.1}">{v}</text>"#,
                if alerte { " frise__valeur--alerte" } else { "" },
                py - if alerte { 19.0 } else { 12.0 }
            );
            // Dates : toutes s'il y a la place, sinon une sur deux (la dernière toujours).
            if n <= dates_toutes || i % 2 == (n - 1) % 2 {
                let _ = write!(
                    s,
                    r#"<text class="frise__date" x="{px:.1}" y="{:.1}">{}</text>"#,
                    h - 10.0,
                    esc(date)
                );
            }
        }
    }
    s.push_str("</svg>");
    s
}

/// Frise large sur écran large, étroite sur téléphone (le CSS n'en montre qu'une).
fn frise_adaptee(points: &[(String, i16)]) -> String {
    if points.is_empty() {
        return frise(points, Format::Large);
    }
    frise(points, Format::Large) + &frise(points, Format::Etroite)
}

/// Saisies (les plus récentes d'abord) → points de la frise, du plus ancien au plus récent.
fn points_frise(suivis: &[Suivi], max: usize) -> Vec<(String, i16)> {
    let mut points: Vec<(String, i16)> = suivis
        .iter()
        .take(max)
        .map(|s| (date_courte(s.cree_le), s.douleur))
        .collect();
    points.reverse();
    points
}

pub fn connexion(erreur: Option<&str>, info: Option<&str>) -> String {
    let erreur = erreur
        .map(|e| format!(r#"<p class="avis avis--erreur" role="alert">{}</p>"#, esc(e)))
        .unwrap_or_default();
    let info = info
        .map(|i| format!(r#"<p class="avis">{}</p>"#, esc(i)))
        .unwrap_or_default();
    page(
        "Connexion",
        Espace::Connexion,
        false,
        &format!(
            r#"<div class="couverture">
  <div class="couverture__tampons" aria-hidden="true">{confidentiel}</div>
  <section class="couverture__etiquette" aria-labelledby="titre-connexion">
    <h1 class="titre" id="titre-connexion">Espace praticien</h1>
    <p class="sous-titre">Dossiers de suivi, réservés au médecin.</p>
    {info}{erreur}
    <form class="formulaire" method="post" action="/connexion">
      <div class="champ"><label class="champ__libelle" for="identifiant">Identifiant</label>
        <input class="saisie" id="identifiant" name="identifiant" autocomplete="username" required maxlength="100"></div>
      <div class="champ"><label class="champ__libelle" for="mot_de_passe">Mot de passe</label>
        <input class="saisie" id="mot_de_passe" name="mot_de_passe" type="password" autocomplete="current-password" required maxlength="200"></div>
      <button class="bouton" type="submit">Se connecter</button>
    </form>
    <p class="note">Session de 15 minutes d'inactivité, 8 heures au plus. 5 échecs bloquent l'adresse IP 15 minutes.</p>
  </section>
</div>"#,
            confidentiel = tampon("Confidentiel · secret médical", "rouge"),
        ),
    )
}

pub fn tableau_de_bord(patients: &[LignePatient], inscription: &Inscription) -> String {
    let alertes = patients.iter().filter(|p| p.alerte).count();
    let mut fiches = String::new();
    for p in patients {
        let couleur = if p.alerte {
            "rouge"
        } else if p.non_lus > 0 {
            "bleu"
        } else {
            "beige"
        };
        let suivi = if p.non_lus > 0 {
            pluriel(p.non_lus, "nouvelle saisie", "nouvelles saisies")
        } else {
            "à jour".into()
        };
        let derniere = p
            .douleurs
            .last()
            .map(|d| format!("{d}/10"))
            .unwrap_or_else(|| "-".into());
        let points: Vec<(String, i16)> = p.douleurs.iter().map(|d| (String::new(), *d)).collect();
        let marque = if p.alerte {
            tampon(&format!("Douleur {derniere}"), "rouge")
        } else {
            String::new()
        };
        let _ = write!(
            fiches,
            r#"<li class="fiche fiche--{couleur}">
  <a class="fiche__lien" href="/praticien/patients/{id}">
    <span class="fiche__onglet">Dossier n° {id:04}</span>
    <span class="fiche__tete"><span class="fiche__nom">{prenom} {nom}</span><span class="fiche__motif">{motif}</span></span>
    <span class="fiche__frise">{frise}</span>
    <span class="fiche__ligne"><span>Dernière douleur</span><span class="fiche__valeur fiche__douleur">{derniere}</span></span>
    <span class="fiche__ligne"><span>Prochain RDV</span><span class="fiche__valeur">{rdv}</span></span>
    <span class="fiche__ligne"><span>Suivi</span><span class="fiche__valeur fiche__suivi">{suivi}</span></span>
  </a>
  {marque}
</li>"#,
            id = p.id,
            prenom = esc(&p.prenom),
            nom = esc(&p.nom),
            motif = esc(&p.motif),
            frise = frise(&points, Format::Compacte),
            rdv = date(p.rdv),
        );
    }
    if fiches.is_empty() {
        fiches =
            r#"<li class="fiche fiche--vide">Aucun dossier : ouvrez le premier avec le formulaire ci-dessous.</li>"#
                .into();
    }
    let bandeau_alerte = if alertes > 0 {
        format!(
            r#"<div class="liaison" role="alert"><strong>{} :</strong> douleur de 7/10 ou plus signalée, saisie non lue.</div>"#,
            pluriel(alertes as i64, "alerte", "alertes")
        )
    } else {
        String::new()
    };
    page(
        "Mes patients",
        Espace::Patients,
        true,
        &format!(
            r#"<header class="chemise__entete">
  <div><h1 class="titre">Mes patients</h1><p class="sous-titre">{n}, rangés par alerte puis par prochain rendez-vous</p></div>
  {registre}
</header>
{bandeau_alerte}
<ul class="fiches">{fiches}</ul>
<section class="feuille feuille--lignee nouveau" aria-labelledby="titre-nouveau">
  <h2 class="feuille__titre" id="titre-nouveau">Ouvrir un dossier</h2>
  <p class="feuille__chapo">Le lien personnel du patient est généré à la création : 24 octets aléatoires.</p>
  <form class="formulaire" method="post" action="/praticien/patients">
    <div class="formulaire__deux">
      <div class="champ"><label class="champ__libelle" for="prenom">Prénom</label><input class="saisie" id="prenom" name="prenom" required maxlength="80"></div>
      <div class="champ"><label class="champ__libelle" for="nom">Nom</label><input class="saisie" id="nom" name="nom" required maxlength="80"></div>
    </div>
    <div class="champ"><label class="champ__libelle" for="motif">Motif du suivi</label><input class="saisie" id="motif" name="motif" required maxlength="200"></div>
    <div class="champ"><label class="champ__libelle" for="consignes">Consignes pour le patient</label><textarea class="saisie saisie--texte" id="consignes" name="consignes" maxlength="2000"></textarea></div>
    <div class="champ"><label class="champ__libelle" for="prochain_rdv">Prochain rendez-vous</label><input class="saisie" id="prochain_rdv" type="datetime-local" name="prochain_rdv"></div>
    <div><button class="bouton" type="submit">Créer et générer le lien patient</button></div>
  </form>
</section>"#,
            n = pluriel(patients.len() as i64, "dossier suivi", "dossiers suivis"),
            registre = registre(inscription, true),
        ),
    )
}

pub fn fiche_patient(p: &Patient, suivis: &[Suivi], lien_patient: &str, inscription: &Inscription) -> String {
    let mut lignes = String::new();
    for s in suivis {
        let haute = s.douleur >= SEUIL;
        let _ = write!(
            lignes,
            r#"<tr{classe}><td data-libelle="Date">{date}</td><td data-libelle="Douleur" class="douleur">{douleur}/10</td><td data-libelle="Tension">{tension}</td><td data-libelle="Commentaire">{commentaire}</td></tr>"#,
            classe = if haute { r#" class="ligne-alerte""# } else { "" },
            date = date(Some(s.cree_le)),
            douleur = s.douleur,
            tension = s.tension.as_deref().map(esc).unwrap_or_else(|| "-".into()),
            commentaire = if s.commentaire.is_empty() {
                "-".into()
            } else {
                esc(&s.commentaire)
            },
        );
    }
    if lignes.is_empty() {
        lignes = r#"<tr><td colspan="4" class="vide">Pas encore de saisie.</td></tr>"#.into();
    }
    let rdv_valeur = p
        .rdv
        .map(|d| d.format("%Y-%m-%dT%H:%M").to_string())
        .unwrap_or_default();
    let nom = format!("{} {}", p.prenom, p.nom);
    let frise = frise_adaptee(&points_frise(suivis, 14));
    page(
        &nom,
        Espace::Patients,
        true,
        &format!(
            r#"<a class="retour" href="/praticien">← Mes patients</a>
<header class="chemise__entete">
  <div><p class="dossier-numero">Dossier n° {id:04}</p><h1 class="titre titre--machine">{nom}</h1><p class="sous-titre">{motif}</p></div>
  {registre}
</header>
<div class="dossier">
  <section class="feuille frise-bloc" aria-labelledby="titre-frise">
    <h2 class="feuille__titre" id="titre-frise">Frise de la douleur</h2>
    <p class="feuille__chapo">{saisies}, seuil d'alerte à {SEUIL}/10. Ouvrir ce dossier marque les saisies comme lues.</p>
    {frise}
  </section>
  <aside class="carte-rdv" aria-labelledby="titre-carte">
    <p class="carte-rdv__entete"><span id="titre-carte">Carte de rendez-vous</span><span>Suivi patient</span></p>
    <p class="carte-rdv__qui">{nom}</p>
    <p class="carte-rdv__libelle">Prochain rendez-vous</p>
    <p class="carte-rdv__date">{rdv}</p>
    <p class="carte-rdv__libelle">Lien personnel du patient</p>
    <p class="carte-rdv__lien"><a class="lien-patient" href="{lien}">{lien}</a></p>
    <p class="carte-rdv__note">À transmettre au patient par un canal sûr. Le jeton est aléatoire (24 octets) et n'apparaît jamais dans les logs.</p>
  </aside>
</div>
<section class="feuille" aria-labelledby="titre-saisies">
  <h2 class="feuille__titre" id="titre-saisies">Saisies du patient</h2>
  <table class="releve">
    <thead><tr><th>Date</th><th>Douleur</th><th>Tension</th><th>Commentaire</th></tr></thead>
    <tbody>{lignes}</tbody>
  </table>
</section>
<section class="feuille feuille--lignee" aria-labelledby="titre-maj">
  <h2 class="feuille__titre" id="titre-maj">Mettre à jour le suivi</h2>
  <form class="formulaire" method="post" action="/praticien/patients/{id}">
    <div class="champ"><label class="champ__libelle" for="consignes">Consignes</label><textarea class="saisie saisie--texte" id="consignes" name="consignes" maxlength="2000">{consignes}</textarea></div>
    <div class="champ"><label class="champ__libelle" for="prochain_rdv">Prochain rendez-vous</label><input class="saisie" id="prochain_rdv" type="datetime-local" name="prochain_rdv" value="{rdv_valeur}"></div>
    <div><button class="bouton" type="submit">Enregistrer</button></div>
  </form>
</section>"#,
            nom = esc(&nom),
            motif = esc(&p.motif),
            rdv = date(p.rdv),
            lien = esc(lien_patient),
            id = p.id,
            consignes = esc(&p.consignes),
            saisies = pluriel(suivis.len() as i64, "saisie", "saisies"),
            registre = registre(inscription, true),
        ),
    )
}

pub fn espace_patient(p: &Patient, suivis: &[Suivi], merci: bool, inscription: &Inscription) -> String {
    let mut lignes = String::new();
    for s in suivis {
        let _ = write!(
            lignes,
            r#"<li><span class="carnet__date">{}</span><span class="carnet__douleur{}">{}/10</span><span class="carnet__mot">{}</span></li>"#,
            date(Some(s.cree_le)),
            if s.douleur >= SEUIL {
                " carnet__douleur--haute"
            } else {
                ""
            },
            s.douleur,
            esc(&s.commentaire)
        );
    }
    let historique = if lignes.is_empty() {
        r#"<p class="vide">Aucune saisie pour le moment.</p>"#.to_string()
    } else {
        format!(
            r#"{}<ol class="carnet__liste">{lignes}</ol>"#,
            frise_adaptee(&points_frise(suivis, 10))
        )
    };
    let consignes = if p.consignes.trim().is_empty() {
        r#"<span class="vide">Aucune pour le moment.</span>"#.to_string()
    } else {
        esc(&p.consignes).replace('\n', "<br>")
    };
    let merci = if merci {
        r#"<p class="avis avis--merci" role="status">Merci, votre médecin a bien reçu votre saisie.</p>"#
    } else {
        ""
    };
    let mut echelle = String::new();
    for n in 0..=10 {
        let _ = write!(
            echelle,
            r#"<label class="echelle__case echelle__case--{n}"><input type="radio" name="douleur" value="{n}" required><span>{n}</span></label>"#
        );
    }
    page(
        "Mon suivi",
        Espace::Patient,
        false,
        &format!(
            r#"{merci}
<header class="carnet__entete"><h1 class="titre">Bonjour {prenom}</h1><p class="sous-titre">Votre carnet de suivi personnel</p></header>
<div class="carnet__grille">
  <aside class="carte-rdv carte-rdv--patient" aria-labelledby="titre-carte">
    <p class="carte-rdv__entete"><span id="titre-carte">Carte de rendez-vous</span><span>Suivi patient</span></p>
    <p class="carte-rdv__libelle">Votre prochain rendez-vous</p>
    <p class="carte-rdv__date">{rdv}</p>
    <p class="carte-rdv__libelle">Consignes de votre médecin</p>
    <p class="carte-rdv__consignes">{consignes}</p>
  </aside>
  <section class="feuille feuille--lignee saisie-jour" aria-labelledby="titre-jour">
    <h2 class="feuille__titre" id="titre-jour">Comment allez-vous aujourd'hui ?</h2>
    <form class="formulaire" method="post">
      <fieldset class="echelle">
        <legend class="champ__libelle">Niveau de douleur, de 0 à 10</legend>
        <div class="echelle__cases">{echelle}</div>
        <p class="echelle__bornes" aria-hidden="true"><span>aucune</span><span>insupportable</span></p>
      </fieldset>
      <div class="champ"><label class="champ__libelle" for="tension">Tension (facultatif, ex. 12/8)</label><input class="saisie" id="tension" name="tension" maxlength="20" inputmode="numeric"></div>
      <div class="champ"><label class="champ__libelle" for="commentaire">Un mot pour votre médecin</label><textarea class="saisie saisie--texte" id="commentaire" name="commentaire" maxlength="1000"></textarea></div>
      <div><button class="bouton" type="submit">Envoyer à mon médecin</button></div>
    </form>
    <p class="urgence">En cas d'urgence, appelez le 15.</p>
  </section>
</div>
<section class="feuille" aria-labelledby="titre-historique">
  <h2 class="feuille__titre" id="titre-historique">Mes dernières saisies</h2>
  {historique}
</section>
<p class="carnet__trace">Chaque ouverture de cette page est inscrite au registre des accès de votre médecin : n° {numero}, à {heure}.</p>"#,
            prenom = esc(&p.prenom),
            rdv = date(p.rdv),
            numero = numero(inscription.numero),
            heure = esc(&inscription.heure),
        ),
    )
}

/// Famille d'action pour la couleur de l'encre dans le registre.
fn famille(action: &str) -> &'static str {
    if action.contains("refusee") || action.contains("bloquee") {
        "rouge"
    } else if action.starts_with("connexion") || action == "deconnexion" {
        "bleu"
    } else if action.starts_with("saisie") || action.starts_with("creation") || action.starts_with("mise_a_jour") {
        "vert"
    } else {
        "encre"
    }
}

pub fn journal(entrees: &[EntreeJournal], total: i64, inscription: &Inscription) -> String {
    let mut lignes = String::new();
    for e in entrees {
        let patient = match (e.patient_id, &e.patient) {
            (Some(id), Some(nom)) => format!(r#"<a class="lien" href="/praticien/patients/{id}">{}</a>"#, esc(nom)),
            (Some(id), None) => format!("#{id}"),
            _ => String::new(),
        };
        let courante = e.numero == inscription.numero;
        let _ = write!(
            lignes,
            r#"<tr{classe}><td class="livre__num" data-libelle="N°">{num}</td><td class="livre__quand" data-libelle="Quand">{date}</td><td data-libelle="Qui">{acteur}</td><td data-libelle="Quoi"><span class="action action--{famille}">{action}</span>{ici}</td><td class="livre__patient" data-libelle="Patient">{patient}</td><td class="livre__ip" data-libelle="IP">{ip}</td></tr>"#,
            classe = if courante { r#" class="livre__courante""# } else { "" },
            num = numero(e.numero),
            date = esc(&e.quand),
            acteur = esc(&e.acteur),
            famille = famille(&e.action),
            action = esc(&e.action),
            ici = if courante {
                r#"<span class="livre__ici">cette page</span>"#
            } else {
                ""
            },
            ip = esc(e.ip.as_deref().unwrap_or("-")),
        );
    }
    if lignes.is_empty() {
        lignes = r#"<tr><td colspan="6" class="vide">Registre vide.</td></tr>"#.into();
    }
    page(
        "Journal d'accès",
        Espace::Journal,
        true,
        &format!(
            r#"<header class="chemise__entete">
  <div><h1 class="titre">Registre des accès</h1><p class="sous-titre">{total_txt}, les 200 dernières affichées. Qui, quoi, quel patient, quand, depuis quelle IP.</p></div>
  {ajout_seul}
</header>
<section class="livre" aria-label="Journal d'accès">
  <table class="livre__table">
    <thead><tr><th>N°</th><th>Quand (Paris)</th><th>Qui</th><th>Quoi</th><th>Patient</th><th>IP</th></tr></thead>
    <tbody>{lignes}</tbody>
  </table>
  <p class="livre__pied">Aucune ligne ne peut être raturée : UPDATE, DELETE et TRUNCATE sont refusés par PostgreSQL lui-même (triggers <code>journal_acces_ajout_seul</code> et <code>journal_acces_sans_truncate</code>).</p>
</section>"#,
            total_txt = pluriel(total, "entrée", "entrées"),
            ajout_seul = tampon("Ajout seul", "rouge"),
        ),
    )
}

/// Case cochée à la main dans la grille du rapport.
fn case(s: Statut) -> &'static str {
    match s {
        Statut::Ok => {
            r#"<svg class="case case--ok" viewBox="0 0 28 24" aria-hidden="true"><rect x="2" y="4" width="18" height="18" rx="1.5"/><path d="M5.5 13.5l4.5 4.8L25 2.5"/></svg><span class="case__mention">Conforme</span>"#
        }
        Statut::Ko => {
            r#"<svg class="case case--ko" viewBox="0 0 28 24" aria-hidden="true"><rect x="2" y="4" width="18" height="18" rx="1.5"/><path d="M5 7.5l12.5 12M17.5 7l-12 12.5"/></svg><span class="case__mention">Non conforme</span>"#
        }
        Statut::Info => {
            r#"<svg class="case case--info" viewBox="0 0 28 24" aria-hidden="true"><rect x="2" y="4" width="18" height="18" rx="1.5"/><path d="M7 13h8"/></svg><span class="case__mention">Pour information</span>"#
        }
    }
}

fn classe_statut(s: Statut) -> &'static str {
    match s {
        Statut::Ok => "ok",
        Statut::Ko => "ko",
        Statut::Info => "info",
    }
}

pub fn conformite(c: &Conformite, praticien: bool) -> String {
    let mut rubriques = String::new();
    let mut n = 0;
    for (i, s) in c.sections.iter().enumerate() {
        let mut lignes = String::new();
        for v in &s.verifications {
            n += 1;
            let _ = write!(
                lignes,
                r#"<li class="controle controle--{statut}"><span class="controle__num">{n}</span><span class="controle__libelle">{libelle}</span><span class="controle__preuve">{valeur}</span><span class="controle__resultat">{case}</span></li>"#,
                statut = classe_statut(v.statut),
                libelle = esc(&v.libelle),
                valeur = esc(&v.valeur),
                case = case(v.statut),
            );
        }
        let _ = write!(
            rubriques,
            r#"<section class="rubrique"><h2 class="rubrique__titre"><span class="rubrique__num">{}</span>{}</h2><ol class="controles">{lignes}</ol></section>"#,
            i + 1,
            esc(s.titre)
        );
    }
    let mut plateforme = String::new();
    for p in &c.plateforme {
        let _ = write!(
            plateforme,
            r#"<li class="controle controle--script"><span class="controle__num" aria-hidden="true"></span><span class="controle__libelle">{}</span><span class="controle__preuve controle__preuve--texte">{}</span><span class="controle__resultat"><span class="case-script">verif-hds.sh</span></span></li>"#,
            esc(p.point),
            esc(p.verification)
        );
    }
    let s = &c.synthese;
    let controles = s.ok + s.ko;
    let (couleur, verdict) = if s.ko == 0 {
        ("vert", "Conforme")
    } else {
        ("rouge", "Non conforme")
    };
    let jour = c.genere_le.split(' ').next().unwrap_or_default();
    let bilan = format!(
        "{}, {}, {}.",
        pluriel(s.ok as i64, "point conforme", "points conformes"),
        pluriel(s.ko as i64, "non conforme", "non conformes"),
        pluriel(s.info as i64, "pour information", "pour information"),
    );
    let note_locale = if !production() && s.ko > 0 {
        r#"<p class="verdict__note">En local, la base n'est pas jointe en TLS. Sur Clever Cloud, <code>APP_ENV=production</code> impose le TLS vers la base, et la ligne 1 passe au vert.</p>"#
    } else {
        ""
    };
    page(
        "Conformité HDS",
        Espace::Conformite,
        praticien,
        &format!(
            r#"<article class="rapport">
  <header class="rapport__tete">
    <p class="rapport__emetteur"><span>Suivi patient</span><span>Auto-inspection HDS</span></p>
    <h1 class="rapport__titre">Rapport d'inspection</h1>
    <p class="rapport__objet">Bonnes pratiques d'hébergement de données de santé, vérifiées à l'instant à partir de cette requête et de la base PostgreSQL. Page publique, sans aucune donnée patient.</p>
    <dl class="rapport__meta">
      <div><dt>Établi le</dt><dd>{genere} (Paris)</dd></div>
      <div><dt>Mode</dt><dd>{mode}</dd></div>
      <div><dt>Version JSON</dt><dd><a class="lien" href="/conformite.json">/conformite.json</a></dd></div>
    </dl>
    <div class="verdict verdict--{couleur}">
      <p class="tampon-verdict" aria-label="Verdict : {verdict}, {ok} sur {controles}"><span class="tampon-verdict__mot">{verdict}</span><span class="tampon-verdict__compte">{ok} / {controles}</span><span class="tampon-verdict__date">{jour}</span></p>
      <div><p class="verdict__bilan">{bilan}</p>{note_locale}</div>
    </div>
  </header>
  <div class="rapport__colonnes" aria-hidden="true"><span>N°</span><span>Point de contrôle</span><span>Preuve relevée</span><span>Résultat</span></div>
  {rubriques}
  <section class="rubrique rubrique--plateforme">
    <h2 class="rubrique__titre"><span class="rubrique__num">{n_plateforme}</span>Côté plateforme Clever Cloud</h2>
    <p class="rubrique__chapo">Ces points ne se lisent pas depuis l'application : <code>scripts/verif-hds.sh</code> les vérifie avec clever-tools et l'API Clever Cloud, sans afficher aucun secret.</p>
    <ol class="controles">{plateforme}</ol>
  </section>
  <footer class="visa">
    <div class="visa__bloc">
      <p class="visa__libelle">Visa</p>
      <p class="visa__signature">Suivi patient, le {genere}</p>
      <p class="visa__note">Rapport recalculé à chaque chargement de la page, par l'application elle-même.</p>
    </div>
    <a class="sceau" href="https://academy.clever.cloud">
      {badge}
      <span class="sceau__texte">{ACADEMIE_TEXTE}</span>
      <span class="sceau__lien">academy.clever.cloud</span>
    </a>
  </footer>
</article>"#,
            genere = esc(&c.genere_le),
            mode = esc(&c.mode),
            ok = s.ok,
            jour = esc(jour),
            n_plateforme = c.sections.len() + 1,
            badge = badge("sceau__badge"),
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frise_entoure_les_douleurs_au_dessus_du_seuil() {
        let points = vec![("1 nov.".to_string(), 4), ("2 nov.".to_string(), 8)];
        let svg = frise(&points, Format::Large);
        assert_eq!(svg.matches("frise__point--alerte").count(), 1);
        assert_eq!(svg.matches("frise__cercle").count(), 1);
        assert!(svg.contains("8/10 le 2 nov."));
        assert!(frise(&[], Format::Etroite).contains("Pas encore de saisie"));
    }
}
