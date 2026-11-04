# CLAUDE.md — demo-rust-hds

Suivi patient en Rust (axum + sqlx), à déployer en zone HDS sur Clever Cloud. La procédure de déploiement est dans le README, section « Déployer en zone HDS » : la suivre dans l'ordre, sans étape ajoutée.

## Règles Clever Cloud

- Toujours `--org <orga>` : jamais l'espace personnel. Demander l'organisation si elle n'est pas donnée.
- App et base dans la même organisation et la même zone : `parhds`.
- `clever scale --build-flavor M` **avant** le premier `clever deploy` (désactivé par défaut ; sans lui le build sature l'instance nano).
- Base : `postgresql-addon`, plan `xxs_sml`, `--option encryption=true` à la création (définitif), puis `clever service link-addon`.
- `clever deploy` pousse le commit courant : `git status` propre avant. Ne jamais commiter `.clever.json` ni `.env`.
- `clever logs` toujours borné : `--since 15m --until now`.
- Si `clever deploy` reste plus de 8 minutes sur « Waiting for deployment to start » : l'arrêter, `clever restart` une fois.
- Ne supprimer aucune ressource Clever sans accord explicite.

## Règles HDS

- Jamais de secret à l'écran : variables listées par leur nom (`clever env | grep -oE '^[A-Za-z_][A-Za-z0-9_]*='`), mot de passe généré dans un fichier, jamais `clever env import`.
- L'app lit `POSTGRESQL_ADDON_URI` ; ne jamais la recopier dans `DATABASE_URL`.
- Données fictives uniquement (`SEED_DEMO=true`). Aucun nom, commentaire ni jeton dans les logs.
- Preuve finale : `./scripts/verif-hds.sh <app>` tout vert, puis `/conformite` (TLS base avec version et chiffrement).

## Points techniques

- `APP_ENV=production` impose : redirection HTTPS (sauf `/health`), cookie `__Host-session` Secure, mot de passe ≥ 12 caractères, TLS vers la base. Sinon l'app refuse de démarrer.
- Pas de feature `macros` de sqlx : elle recompile sqlx, rustls et ring une deuxième fois (build propre 34-41 s au lieu de 55 s sur un Mac M). Les migrations sont embarquées par `include_str!` dans `src/migrations.rs` (même moteur que `sqlx::migrate!`) ; une nouvelle migration = un fichier dans `migrations/` + une ligne dans `MIGRATIONS`.
- Routes axum 0.8 : `/p/{jeton}`, jamais `:jeton` (panique au démarrage).
- Requêtes SQL paramétrées uniquement ; dates converties à l'heure de Paris par PostgreSQL (`AT TIME ZONE 'Europe/Paris'`).
- IP client : dernière valeur de la **dernière ligne** `X-Forwarded-For` (ajoutée par le proxy Clever), validée comme adresse IP, sinon l'adresse de la socket. Jamais la première valeur ni une ligne envoyée par le client (tests dans `src/auth.rs`).
- `static/cc-brand.css` et `static/cc-badge.svg` sont des copies du kit de marque : ne jamais les modifier, styles propres dans `static/app.css`. Pas de Google Fonts : aucune ressource tierce sur une app de santé (polices système : Seravek, American Typewriter, DIN Condensed, avec repli).
- Interface « dossier médical papier » : chemise à onglets, fiches, carte de rendez-vous, registre numéroté, `/conformite` en rapport d'inspection tamponné. La CSP interdit tout script et tout `style="…"` : dessins en SVG (attributs de présentation), aucune ligne de JS.
- `scripts/verif-hds.sh` lit `.clever.json` dans le dossier de la démo, sinon à la racine git (déploiement depuis un catalogue avec `APP_FOLDER`) ; un ID `app_…` marche sans `.clever.json`. `bash -n` et `shellcheck` doivent passer.
- Le blocage après 5 échecs est en mémoire : propre à chaque instance, remis à zéro au redémarrage.

## Mesures (preuve du 2026-11-04, app `demo-rust-hds-proof`)

- `clever deploy` complet : 90 s. File d'attente ~11 s, build 41 s (cargo 27 s sur M, 20 s d'envoi du cache de build de 985 Mo et du cache de dépendances), démarrage + `/health` 10 s.
- Au démarrage : base jointe en TLSv1.3 (`TLS_AES_256_GCM_SHA384`), PostgreSQL 18.6, migrations et seed en moins d'une seconde.
- `force-https` était déjà `ENABLED` avant l'étape 5 (vraisemblablement le défaut en `parhds`) ; la commande du README reste, elle rend le réglage explicite et vérifiable.
- Depuis le Mac, l'add-on `parhds` répond (`pg_isready` OK) et une copie `pg_dump | psql` en TLS a recopié 50/50 lignes (tests du 2026-11-04). Pour la vidéo, `/conformite` reste la preuve la plus rapide, sans manipuler d'identifiants.
- Certificat TLS de l'add-on autosigné : `sslmode=require` chiffre sans vérifier la chaîne. Il n'existe pas de variable `POSTGRESQL_CA_CERT` ; `verify-full` ne marche qu'en épinglant ce certificat à la main.
- `/conformite` sur Clever Cloud (parhds, 2026-11-04) : 21 OK, 0 KO, 1 info (version de PostgreSQL).
