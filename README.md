# Suivi patient en Rust, déployé en zone HDS sur Clever Cloud

Démo avant-vente : une application de suivi patient écrite en Rust, déployée sur Clever Cloud dans la zone **HDS** (hébergement de données de santé), avec les bonnes pratiques HDS visibles et vérifiables.

> **Données fictives uniquement.** Une mise en production de vraies données de santé suppose le contrat HDS signé avec Clever Cloud.

## Ce que montre la démo

- **Un déploiement natif** : runtime Rust de Clever Cloud, sans Dockerfile, un `git push` via `clever deploy`.
- **La zone HDS de bout en bout** : application et base PostgreSQL dans `parhds`, base dédiée chiffrée au repos et sauvegardée, HTTPS forcé.
- **Des preuves en direct** :
  - la page publique `/conformite` (et `/conformite.json`) lit l'état réel de l'application : TLS vers la base lu dans `pg_stat_ssl`, protocole HTTPS de la requête, en-têtes, session, trigger du journal ;
  - le script `scripts/verif-hds.sh` vérifie la plateforme : zone, chiffrement, sauvegardes, HTTPS forcé, 2FA de l'organisation.
- **L'application** reprend la démo Node `suivi-patient` :
  - espace praticien (connexion) : tableau des patients et prochains rendez-vous, alerte douleur ≥ 7/10, fiche avec historique, lien personnel et consignes, création d'un patient ;
  - espace patient par lien personnel `/p/<jeton>` (24 octets aléatoires) : consignes, rendez-vous, saisie douleur / tension / commentaire ;
  - journal d'accès en base, en ajout seul, consultable sur `/journal`.

Stack : Rust, axum 0.8, sqlx 0.8 (PostgreSQL, TLS rustls), un seul binaire. HTML rendu côté serveur, kit de marque Clever, aucune ressource tierce (ni police ni script externe).

## Déployer en zone HDS

Prérequis : `clever login`, une organisation Clever Cloud avec la 2FA pour tous ses membres, ce dépôt cloné avec au moins un commit.

```bash
ORG=orga_xxx                                   # l'organisation, jamais l'espace personnel
APP=suivi-patient-hds
MDP_FICHIER="$HOME/.suivi-patient-hds.mdp"     # mot de passe praticien, hors du dépôt
```

**1. Créer l'application Rust dans la zone HDS.** Les données de santé ne quittent pas une zone certifiée HDS : `parhds` est à Paris.

```bash
clever create --type rust --region parhds --org "$ORG" "$APP"
```

**2. Dimensionner.** Le build tourne sur une instance dédiée M, séparée de l'instance qui sert les patients ; l'application démarre en nano et monte seule jusqu'à XS et 2 instances pour rester disponible.

```bash
clever scale --build-flavor M
clever scale --flavor nano
clever scale --min-flavor nano --max-flavor XS --min-instances 1 --max-instances 2
```

**3. Créer la base PostgreSQL et la lier.** Plan dédié (pas le plan DEV partagé), même zone HDS, chiffrement au repos choisi à la création (impossible à ajouter ensuite), sauvegardes quotidiennes incluses.

```bash
clever addon create postgresql-addon "$APP-db" --plan xxs_sml --region parhds \
  --option encryption=true --org "$ORG" --yes
clever service link-addon "$APP-db"
```

**4. Poser les variables, une par une.** Les secrets vivent dans les variables de l'application, jamais dans git ; le mot de passe est généré et n'apparaît pas à l'écran.

```bash
clever env set APP_ENV production
clever env set SEED_DEMO true
clever env set PRATICIEN_LOGIN admin
(umask 077 && openssl rand -hex 16 > "$MDP_FICHIER")
clever env set PRATICIEN_PASSWORD "$(cat "$MDP_FICHIER")" > /dev/null
```

`POSTGRESQL_ADDON_URI` est injectée par la liaison : l'application la lit directement, ne la recopiez dans aucune autre variable.

**5. Forcer HTTPS au niveau de la plateforme.** Aucune donnée ne circule en clair : le proxy Clever Cloud redirige tout accès HTTP vers HTTPS.

```bash
clever config set force-https true
```

**6. Cache de build et contrôle de santé.** Les dépendances compilées sont réutilisées d'un déploiement à l'autre, et un déploiement n'est validé que si l'application répond et joint sa base.

```bash
clever env set CC_CACHE_DEPENDENCIES true
clever env set CC_HEALTH_CHECK_PATH /health
```

**7. Déployer.** Clever Cloud compile le commit courant (`cargo build --release --locked`), démarre le binaire, joue les migrations et crée les 3 patients fictifs.

```bash
clever deploy
clever domain
```

**8. Vérifier.**

```bash
./scripts/verif-hds.sh "$APP"     # checklist ✅/❌, sans aucun secret
```

Puis ouvrir `https://<domaine>/conformite`, et `https://<domaine>/connexion` avec `admin` et le mot de passe de `$MDP_FICHIER` (`pbcopy < "$MDP_FICHIER"` sur macOS).

### Durées mesurées

Preuve du 4 novembre 2026 (app `demo-rust-hds-proof`, zone `parhds`, build dédié M), en suivant cette section à la lettre :

| Étape | Durée |
|---|---|
| `clever create` | 1 s |
| `clever scale` (3 commandes) | 2 s |
| `clever addon create` + `link-addon` | 2 s (base `ACTIVE`, chiffrée, sauvegardée) |
| Variables et `force-https` | 5 s |
| Premier build Clever Cloud | 41 s, dont `cargo build --release --locked` 27 s et 20 s d'envoi des caches |
| `clever deploy` complet (push, build, démarrage, `/health`) | 1 min 30 s |

Build propre en local (Mac M, 194 crates) : 42 s.

## Bonnes pratiques HDS

| Pratique | Comment | Où on le voit |
|---|---|---|
| Zone HDS | `--region parhds` pour l'application et la base | `verif-hds.sh` (zone avec le tag `contract:hds`), Console |
| Plan dédié, pas DEV | `--plan xxs_sml` : instance dédiée, 45 connexions, sauvegardes. Le plan DEV est partagé, sans chiffrement ni sauvegarde | `verif-hds.sh`, Console de l'add-on |
| Chiffrement au repos | `--option encryption=true` à la création de la base, définitif | `verif-hds.sh` (API v4 addon-providers) |
| TLS partout | Base en `sslmode=require` (l'app refuse `PG_SSL=disable` en production) ; HTTPS côté navigateur | `/conformite` : version TLS et chiffrement lus dans `pg_stat_ssl` |
| HTTPS forcé + HSTS | `clever config set force-https true`, redirection 301 aussi dans l'app, HSTS un an | `verif-hds.sh` (curl http → 301), `/conformite` |
| En-têtes de sécurité | CSP stricte (`default-src 'none'`, `frame-ancestors 'none'`), `nosniff`, `Referrer-Policy`, `Cache-Control: no-store` | `/conformite`, `curl -I` |
| Pas d'accès direct aux instances | Seul le load balancer Clever Cloud est exposé ; pas d'accès direct host / direct port ; interventions par les mécanismes de la plateforme (Console, `clever ssh`, authentifiés) | Console |
| 2FA pour tous les membres | Activée sur chaque compte de l'organisation | `verif-hds.sh` (API organisations/members) |
| Moindre privilège | Rôles de l'organisation : Développeur pour déployer, Comptable pour la facturation, Admin réservé | Console → Membres |
| Secrets en variables | `clever env set`, mot de passe généré sans affichage, `.env` ignoré par git | `clever env` (noms seulement) |
| Logs sans donnée de santé | Une ligne par requête : méthode, chemin, statut, durée. Jeton `/p/…` masqué, jamais de nom ni de commentaire, erreurs SQL réduites au code SQLSTATE | `clever logs`, `/conformite` |
| Journal d'accès | Table `journal_acces` (qui, quoi, quel patient, quand, IP), en ajout seul : un trigger refuse UPDATE, DELETE et TRUNCATE | `/journal`, `/conformite` (`pg_trigger`) |
| Sessions courtes | Session serveur, 15 min d'inactivité, 8 h au plus ; cookie `HttpOnly`, `SameSite=Strict`, `Secure` (`__Host-`) ; 5 échecs par IP bloquent 15 min ; CSRF refusé | `/conformite` |
| Sauvegardes quotidiennes | Incluses dans le plan dédié (7 conservées) | `verif-hds.sh`, Console → Backups |
| Domaine personnalisé + contrat HDS | Pas de production sur `cleverapps.io` ; contrat HDS signé avant toute donnée réelle | Voir « Pour la production » |

Le certificat TLS de l'add-on PostgreSQL est autosigné : le mode `require` chiffre la connexion sans vérifier la chaîne de certificats. Pour aller plus loin, épingler le certificat de l'add-on (mode `verify-ca` avec ce certificat comme autorité).

## Lancer en local

Avec un PostgreSQL local sans TLS et Rust 1.85 ou plus :

```bash
createdb suivi_patient_hds
cp .env.example .env          # valeurs factices, admin / admin
set -a && . ./.env && set +a
cargo run --release           # http://localhost:8080
```

En local, `admin` / `admin` est accepté et un bandeau « Version locale » s'affiche. `PG_SSL=disable` n'est accepté que hors production.

## Pour la production

1. **Contrat HDS** signé avec Clever Cloud (périmètre, rôles hébergeur et responsable de traitement) avant toute donnée réelle.
2. **Domaine personnalisé** : `clever domain add suivi.mon-cabinet.fr`, CNAME vers Clever Cloud, certificat automatique.
3. **Authentification nominative** : un compte par praticien avec second facteur (Pro Santé Connect ou OpenID Connect) au lieu du compte unique.
4. **Liens patients** à durée de vie limitée et révocables, transmis par un canal maîtrisé (MSSanté).
5. **Journal d'accès** conservé selon la durée réglementaire et exporté hors de la base (SIEM) ; rôle PostgreSQL distinct pour l'application, sans droit sur le trigger.
6. **Base** dimensionnée à l'usage, rétention des sauvegardes alignée sur l'analyse d'impact (AIPD), épinglage du certificat PostgreSQL.
7. **Résilience** : 2 instances minimum (`clever scale --min-instances 2`), alertes.
8. **Anti force brute partagé** : le compteur d'échecs est en mémoire, propre à chaque instance ; le partager (Redis HDS) au-delà d'une instance.

## Structure

```
src/main.rs         configuration, middleware (HTTPS, CSRF, en-têtes, logs), routes
src/auth.rs         connexion, sessions, blocage après 5 échecs
src/pages.rs        espace praticien, espace patient, journal, données de démo
src/conformite.rs   vérifications de /conformite et /conformite.json
src/html.rs         pages HTML (format! + échappement)
src/migrations.rs   migrations embarquées dans le binaire
migrations/         schéma SQL (dont le trigger d'ajout seul)
static/             kit de marque Clever (cc-brand.css, logo) et app.css
scripts/verif-hds.sh  checklist HDS côté plateforme
```
