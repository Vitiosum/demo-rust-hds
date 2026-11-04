#!/usr/bin/env bash
# Checklist HDS d'une application Clever Cloud et de sa base PostgreSQL liée.
#
#   ./scripts/verif-hds.sh <alias|app_id>
#
# Lit l'API Clever Cloud avec les identifiants de clever-tools (clever curl) et l'app elle-même
# avec curl. Aucun secret n'est affiché : chaque réponse est réduite par jq aux seuls champs utiles.
# Code de sortie : 0 si tout est vert, 1 sinon.
set -euo pipefail

API=https://api.clever-cloud.com
DOSSIER=$(cd "$(dirname "$0")/.." && pwd)
OK=0
KO=0

usage() {
  echo "Usage : $0 <alias|app_id>   (alias de .clever.json ou ID app_…)" >&2
  exit 2
}

[ $# -eq 1 ] || usage
CIBLE=$1
for outil in clever jq curl; do
  command -v "$outil" >/dev/null || { echo "Outil manquant : $outil" >&2; exit 2; }
done

# GET sur l'API Clever Cloud ; le résultat ne sort jamais tel quel, il passe toujours par jq.
api() {
  clever curl -s "$API/$1" 2>/dev/null
}

ligne() { # statut libellé détail
  local icone
  case $1 in
    ok) icone="✅"; OK=$((OK + 1)) ;;
    ko) icone="❌"; KO=$((KO + 1)) ;;
    *) icone="ℹ️ " ;;
  esac
  printf '%s %s\n' "$icone" "$2"
  if [ -n "${3:-}" ]; then printf '   %s\n' "$3"; fi
}

titre() { printf '\n%s\n' "$1"; }

# --- Application : alias de .clever.json, sinon ID cherché dans les organisations du compte
APP_ID=""
ORG=""
if [ -f "$DOSSIER/.clever.json" ]; then
  read -r APP_ID ORG < <(jq -r --arg c "$CIBLE" \
    '.apps[] | select(.alias == $c or .app_id == $c or .name == $c) | "\(.app_id) \(.org_id)"' \
    "$DOSSIER/.clever.json" | head -1) || true
fi
if [ -z "$APP_ID" ] && [[ $CIBLE == app_* ]]; then
  for o in $(api v2/organisations | jq -r '.[].id'); do
    if [ "$(api "v2/organisations/$o/applications/$CIBLE" | jq -r '.id // empty')" = "$CIBLE" ]; then
      APP_ID=$CIBLE
      ORG=$o
      break
    fi
  done
fi
[ -n "$APP_ID" ] || { echo "Application introuvable : $CIBLE (alias de .clever.json ou ID app_…)" >&2; exit 2; }

read -r NOM ZONE FORCE_HTTPS DOMAINE < <(api "v2/organisations/$ORG/applications/$APP_ID" |
  jq -r '"\(.name) \(.zone) \(.forceHttps) \(.vhosts[0].fqdn // "" | rtrimstr("/"))"')
TAGS=$(api v4/products/zones | jq -r --arg z "$ZONE" '.[] | select(.name == $z) | .tags | join(",")')

echo "Vérification HDS · $NOM ($APP_ID)"
echo "Organisation $ORG · https://$DOMAINE"

# --- Zone et données
titre "Zone et données"
if [[ $ZONE == *hds && ,$TAGS, == *,contract:hds,* ]]; then
  ligne ok "Application en zone $ZONE" "zone certifiée HDS (tag contract:hds de l'API zones)"
else
  ligne ko "Application en zone $ZONE" "attendu : une zone HDS (parhds)"
fi

read -r ADDON_ID ADDON_NOM ADDON_ZONE < <(api "v2/organisations/$ORG/applications/$APP_ID/addons" |
  jq -r '[.[] | select(.provider.id == "postgresql-addon")][0] // {} | "\(.id // "-") \(.name // "-") \(.region // "-")"')
if [ "$ADDON_ID" = "-" ]; then
  ligne ko "Add-on PostgreSQL lié" "aucun add-on PostgreSQL lié à l'application"
else
  # Détail de l'add-on (contient des identifiants) : seuls statut, zone, plan, version et options sont extraits.
  read -r STATUT PLAN VERSION CHIFFREMENT SAUVEGARDE < <(api "v4/addon-providers/postgresql-addon/addons/$ADDON_ID" |
    jq -r '"\(.status) \(.plan) \(.version) " +
      ([.features[] | select(.name == "encryption") | .enabled][0] // false | tostring) + " " +
      ([.features[] | select(.name == "do-backup") | .enabled][0] // false | tostring)')
  if [ "$ADDON_ZONE" = "$ZONE" ] && [[ $ADDON_ZONE == *hds ]] && [ "$PLAN" != "dev" ]; then
    ligne ok "Add-on PostgreSQL $ADDON_NOM lié, zone $ADDON_ZONE" "plan $PLAN (dédié), PostgreSQL $VERSION, $STATUT"
  else
    ligne ko "Add-on PostgreSQL $ADDON_NOM, zone $ADDON_ZONE" "attendu : même zone HDS que l'app, plan dédié (pas dev) ; plan $PLAN"
  fi

  if [ "$CHIFFREMENT" = "true" ]; then
    ligne ok "Chiffrement au repos actif" "option encryption de l'add-on (API v4 addon-providers)"
  else
    ligne ko "Chiffrement au repos inactif" "il ne s'active qu'à la création : recréer la base avec --option encryption=true"
  fi

  REAL_ID=$(api "v2/organisations/$ORG/addons/$ADDON_ID" | jq -r '.realId')
  POLITIQUE=$(api v2/products/addonproviders/postgresql-addon |
    jq -r --arg p "$PLAN" '.plans[] | select(.slug == $p) | .features[] | select(.name == "Backups") | .value')
  NB=$(api "v2/backups/$ORG/$REAL_ID" | jq -r 'if type == "array" then length else 0 end')
  if [ "$SAUVEGARDE" = "true" ] && [ -n "$POLITIQUE" ] && [ "$POLITIQUE" != "No" ]; then
    DETAIL="option do-backup active, plan « $POLITIQUE », $NB sauvegarde(s) disponible(s)"
    [ "$NB" -gt 0 ] || DETAIL="$DETAIL : la première est faite dans les 24 h suivant la création"
    ligne ok "Sauvegardes actives" "$DETAIL"
  else
    ligne ko "Sauvegardes inactives" "do-backup=$SAUVEGARDE, plan « ${POLITIQUE:-inconnu} »"
  fi
fi

# --- Accès
titre "Accès"
if [ "$FORCE_HTTPS" = "ENABLED" ]; then
  ligne ok "HTTPS forcé par la plateforme" "force-https activé (clever config set force-https true)"
else
  ligne ko "HTTPS non forcé" "clever config set force-https true"
fi

read -r CODE CIBLE_REDIR < <(curl -s -o /dev/null -m 15 -w '%{http_code} %{redirect_url}\n' "http://$DOMAINE/conformite") || true
if [ "${CODE:-}" = "301" ] && [[ ${CIBLE_REDIR:-} == https://* ]]; then
  ligne ok "Redirection http → https" "http://$DOMAINE/conformite → $CODE $CIBLE_REDIR"
else
  ligne ko "Redirection http → https" "obtenu : ${CODE:-aucune réponse} ${CIBLE_REDIR:-}"
fi

HSTS=$(curl -sI -m 15 "https://$DOMAINE/health" | tr -d '\r' | sed -n 's/^[Ss]trict-[Tt]ransport-[Ss]ecurity: //p' | head -1) || true
if [[ $HSTS == *max-age=* ]]; then
  ligne ok "HSTS envoyé" "Strict-Transport-Security: $HSTS"
else
  ligne ko "HSTS absent" "en-tête Strict-Transport-Security non reçu sur https://$DOMAINE/health"
fi

read -r SSL TLS CHIFFRE < <(curl -s -m 15 "https://$DOMAINE/conformite.json" |
  jq -r '"\(.base.ssl) \(.base.version_tls // "-") \(.base.chiffrement // "-")"' 2>/dev/null) || true
if [ "${SSL:-}" = "true" ]; then
  ligne ok "Connexion à la base en TLS" "$TLS · $CHIFFRE (pg_stat_ssl, lu dans /conformite.json)"
else
  ligne ko "Connexion à la base en TLS" "ssl=${SSL:-illisible} dans https://$DOMAINE/conformite.json"
fi

# --- Organisation
titre "Organisation"
read -r TOTAL SANS_2FA < <(api "v2/organisations/$ORG/members" |
  jq -r '"\(length) \([.[] | select((.member.preferredMFA // "NONE") == "NONE")] | length)"')
if [ "$SANS_2FA" = "0" ]; then
  ligne ok "Double authentification" "2FA active pour $TOTAL/$TOTAL membre(s) de l'organisation"
else
  ligne ko "Double authentification" "$SANS_2FA membre(s) sur $TOTAL sans 2FA"
fi
ligne info "Contrat HDS" "à signer avec Clever Cloud avant toute donnée réelle (non lisible par l'API)"

echo
if [ "$KO" -eq 0 ]; then
  echo "Résultat : $OK/$OK vérifications OK"
else
  echo "Résultat : $KO vérification(s) en échec sur $((OK + KO))"
  exit 1
fi
