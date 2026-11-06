# Mnemo 0.1.0

Prise de notes Markdown façon Obsidian, native (Rust), sans Electron. Les notes, tickets et réglages sont stockés
dans PostgreSQL et synchronisés en direct entre tous les appareils connectés à la même base.

## Contenu de l'archive

| Dossier / fichier | Pour |
|---|---|
| `linux/mnemo_0.1.0-1_amd64.deb` | Debian 10+ / Ubuntu 18.04+ : `sudo apt install ./mnemo_0.1.0-1_amd64.deb` |
| `linux/mnemo-linux-x86_64` | tout Linux x86_64 (glibc ≥ 2.28 : Arch, Fedora, Debian, Ubuntu…) |
| `linux/mnemo-linux-aarch64` | Linux ARM 64 bits (Raspberry Pi 4/5, portables ARM) |
| `windows/mnemo.exe` | Windows 10 / 11 (double-clic) |
| `macos/` | instructions : `Mnemo.app` se construit sur un Mac (`scripts/build-macos.sh`) ou via GitHub Actions |
| `source/` | code source complet (avec l'historique git dans `source/.git`) |
| `RAPPORT_TESTS.md` | ce qui a été testé et comment |

## Démarrage rapide

1. Une base PostgreSQL : `docker compose up -d` dans `source/` (ou toute base existante : Neon, Supabase, RDS…).
2. Lancez Mnemo et collez l'URL : `postgres://postgres:postgres@localhost:5432/vault`
   (ajoutez `?sslmode=require` pour un serveur distant).
3. Sur un autre ordinateur, mettez la même URL : tout est synchronisé en direct.

Linux : si une bibliothèque graphique manque, Mnemo affiche la commande `apt` / `pacman` / `dnf` à lancer.
Arch : `cd source/packaging/arch && makepkg -si` construit un paquet natif.

Voir `source/README.md` pour toutes les fonctionnalités (plugins, thèmes CSS Obsidian, tableau de bord, GitHub…).
