#!/usr/bin/env zsh
# Génère les clés de signature Tauri pour l'updater
# Lance ce script dans un terminal interactif

source ~/.zshrc 2>/dev/null || true
cd "$(dirname "$0")"

echo "=== Génération des clés de signature Tauri ==="
echo ""
echo "Tu vas être invité à entrer un mot de passe pour protéger la clé privée."
echo "Ce mot de passe devra être stocké dans le secret GitHub TAURI_PRIVATE_KEY_PASSWORD"
echo ""

mkdir -p ~/.tauri

./node_modules/.bin/tauri signer generate -w ~/.tauri/parler.key

echo ""
echo "=== ÉTAPES SUIVANTES ==="
echo ""
echo "1. Copie la clé PUBLIQUE ci-dessus (ligne 'public key: ...')"
echo "   → Remplace la valeur de 'pubkey' dans src-tauri/tauri.conf.json"
echo ""
echo "2. Ajoute ces secrets dans GitHub (Settings > Secrets > Actions) :"
echo "   TAURI_PRIVATE_KEY  = contenu de ~/.tauri/parler.key"
echo "   TAURI_PRIVATE_KEY_PASSWORD = le mot de passe que tu viens d'entrer"
echo ""
cat ~/.tauri/parler.key.pub 2>/dev/null && echo "" || echo "(clé publique non trouvée)"
