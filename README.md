# Mnemo

Base de connaissances Markdown **native** (Rust + egui, sans Electron, sans JS), dont les notes
sont stockées dans **PostgreSQL** et **synchronisées en direct** entre toutes les instances
connectées à la même base. Un seul binaire pour Windows, macOS et Linux.

## Fonctionnalités

- Éditeur Markdown avec **aperçu en direct** (modes Édition / Divisé / Lecture), coloration du code, cases à cocher cliquables
- `[[Liens wiki]]` avec **autocomplétion** (`[[` + nom), `[[Note|alias]]`, `[[Note#titre]]`, création au clic si la note n'existe pas
- **Rétroliens**, `#tags`, recherche plein texte (Postgres `tsvector`), arborescence de dossiers
- Renommer / déplacer une note **met à jour les liens** dans les autres notes
- **Live refresh** : chaque modification (depuis une autre machine, une autre fenêtre ou même un `UPDATE` SQL) apparaît instantanément
- **Résolution de conflits façon GitHub** : fusion à 3 voies, les changements qui ne se chevauchent pas sont fusionnés automatiquement ; les autres s'affichent côte à côte avec un diff unifié et un choix par bloc
- **Plugins** en [Rhai](https://rhai.rs) (scripting embarqué en Rust, sandboxé), activables et configurables, avec réglages synchronisés via la base
- Palette de commandes (Ctrl+P), sélecteur de notes (Ctrl+O), graphe des liens (Ctrl+G, optionnel), thème clair/sombre
- Interface calquée sur le thème par défaut d'Obsidian : ruban, explorateur avec chevrons et guides, onglets avec historique
  (← →), fil d'Ariane, titre intégré éditable (renomme la note), colonne de lecture de 700 px, barre d'état flottante
- **Live preview** dans l'éditeur : la syntaxe markdown (`**`, `##`, `[[ ]]`…) n'apparaît que sur la ligne du curseur ;
  puces, cases à cocher cliquables, citations et blocs de code dessinés ; Ctrl+clic sur un lien pour l'ouvrir

## Démarrage rapide

1. Démarrez une base PostgreSQL (ou utilisez une base existante : Neon, Supabase, RDS…) :
   ```bash
   docker compose up -d          # PostgreSQL 16 sur localhost:5432 (base "vault")
   ```
2. Lancez Mnemo :
   ```bash
   cargo run --release
   ```
3. Au premier lancement, entrez l'URL de connexion (par défaut `postgres://postgres:postgres@localhost:5432/vault`).
   Le schéma est créé automatiquement. Pour une base distante, ajoutez `?sslmode=require`.
4. Sur un autre ordinateur, entrez la même URL : tout est synchronisé en direct.

Sous Linux, si une bibliothèque graphique manque, Mnemo affiche la commande `apt` / `pacman` / `dnf` à lancer.

Variables d'environnement utiles :

| Variable | Rôle |
|---|---|
| `MNEMO_DATABASE_URL` | URL Postgres (prioritaire sur la config) |
| `MNEMO_CONFIG_DIR` | dossier de config (par défaut le dossier de config de l'OS) |
| `MNEMO_PLUGINS_DIR` | dossier des plugins (par défaut `<config>/plugins`) |

## Documentation

| Document | Contenu |
|---|---|
| [docs/plugins.md](docs/plugins.md) | écrire un plugin Rhai : `plugin.toml`, hooks, API |
| [docs/themes.md](docs/themes.md) | thèmes et snippets CSS au format Obsidian |
| [docs/RAPPORT_TESTS.md](docs/RAPPORT_TESTS.md) | ce qui a été testé et comment |

## Installation / compilation par OS

| OS | Installer | Compiler soi-même |
|---|---|---|
| **Debian / Ubuntu** | `sudo apt install ./mnemo_0.1.0-1_amd64.deb` | `cargo build --release` |
| **Arch Linux** | `cd packaging/arch && makepkg -si` | idem |
| **Autres Linux** | binaire `mnemo-linux-x86_64` / `aarch64` (glibc ≥ 2.28) | idem |
| **Windows 10/11** | `mnemo.exe` | `cargo build --release` (Rust + MSVC) |
| **macOS 11+** (Apple Silicon et Intel) | `scripts/build-macos.sh` → `Mnemo.app` + `.dmg` | idem (Xcode CLT) |

Le workflow `.github/workflows/release.yml` compile les trois OS sur des machines natives (Actions › Run workflow,
ou en poussant un tag `v*`). Le binaire Linux est construit avec `cargo zigbuild` pour glibc 2.28 : il tourne sur
Debian 10+, Ubuntu 18.04+, Arch, Fedora… Bibliothèques nécessaires (présentes sur tout poste de bureau) :

- Debian / Ubuntu : `sudo apt install libx11-6 libxkbcommon-x11-0 libgl1 libegl1 libwayland-client0 libxcursor1 libxrandr2 libxi6`
- Arch : `sudo pacman -S libx11 libxkbcommon-x11 libglvnd mesa wayland libxcursor libxrandr libxi`

## Tableau de bord : projets, tickets et GitHub

Ouvrez-le avec l'icône du ruban ou **Ctrl+Shift+D**. Tout est stocké dans PostgreSQL et synchronisé en direct.

- **Projets** contenant des **tickets** numérotés (T-1, T-2…), avec statut (À faire, En cours, Bloqué, En revue, Terminé),
  priorité (Basse → Urgente), description Markdown et **tags** : Feature, Bug, Amélioration, Documentation,
  Tâche technique, Sécurité, Performance, Recherche, ou vos propres tags.
- **Vue « Lignes »** : todo list par projet ; chaque projet se déplie en lignes de tags, qui se déplient en tickets
  (case à cocher pour terminer, ajout rapide d'un ticket dans chaque tag). Un ticket avec plusieurs tags apparaît dans chacun.
- **Vue « Colonnes »** : kanban par statut, glisser-déposer des cartes.
- **Avancement** : barres de progression globale, par projet et par tag ; compteurs par statut.
- **Rapports** : des notes liées aux tickets. « Nouveau rapport » crée une note pré-remplie dans `Rapports/<Projet>/`,
  ou liez une note existante ; dans une note, l'onglet *Tickets liés* du panneau de droite montre et gère ses tickets.
- **GitHub** (Paramètres › GitHub : token personnel, URL d'API pour GitHub Enterprise) puis, par projet, un dépôt
  `propriétaire/nom` :
  - **Synchroniser** importe les issues (hors pull requests) et applique les changements des deux côtés :
    titre, description, labels ↔ tags (`bug` ↔ Bug, `feature` ↔ Feature, `enhancement` ↔ Amélioration…),
    ouverte/fermée ↔ statut. Le côté modifié depuis la dernière synchro gagne ; si les deux ont changé, le plus récent.
  - **Publier sur GitHub** crée l'issue d'un ticket local ; ensuite chaque modification (statut, titre, tags…) est
    poussée automatiquement. Synchronisation automatique toutes les 5 minutes (désactivable).
  - Le token reste dans le fichier de configuration local (ou `MNEMO_GITHUB_TOKEN`), jamais dans la base.

## Comment marche la synchronisation

```
 Mnemo (PC)  ──┐                       ┌──  Mnemo (Mac)
               │  UPDATE notes …       │
               ▼                       │
          ┌────────────── PostgreSQL ──────────────┐
          │ trigger BEFORE : version+1, updated_by │
          │ trigger AFTER  : liens + pg_notify()   │
          └───────────── NOTIFY vault_events ──────┘
               │                       ▲
               └──── LISTEN ───────────┘  (chaque instance)
```

- Chaque instance garde une connexion `LISTEN vault_events` (reconnexion automatique avec backoff).
- Les triggers (`src/schema.sql`) incrémentent `version` et envoient un `NOTIFY` : même une modification faite en SQL est propagée.
- Sauvegarde automatique 0,6 s après la dernière frappe, avec **verrouillage optimiste** (`base_version`).
- Si la note a changé ailleurs pendant que vous écriviez : fusion à 3 voies (`src/merge.rs`) avec comme base la dernière version
  enregistrée connue. Rien n'est jamais écrasé sans votre accord ; si vous quittez avec un conflit non résolu, votre version
  est conservée dans une note `… (conflit <date>)`.

## Structure

| Fichier | Rôle |
|---|---|
| `src/schema.sql` | tables, triggers de version / liens / NOTIFY |
| `src/db.rs` | worker Postgres (tokio-postgres), LISTEN, transactions |
| `src/app.rs` | interface egui |
| `src/merge.rs` | fusion à 3 voies et diff unifié |
| `src/board.rs` | projets, tickets, tags, plan de synchronisation GitHub |
| `src/github.rs` | client API GitHub (issues) |
| `src/app/dashboard.rs` | interface du tableau de bord |
| `src/plugins.rs` | hôte de plugins Rhai |
| `src/markdown.rs` | wikilinks / tags → liens cliquables |
| `src/graph.rs` | vue graphe (optionnelle) |
| `src/theme.rs` | palette, polices, widgets, coloration / live preview de l'éditeur |
| `src/css.rs` | lecture des thèmes / snippets CSS Obsidian (variables, var(), calc(), couleurs) |
| `assets/plugins/` | plugins d'exemple |
| `assets/fonts/` | Inter (OFL) et icônes Lucide (ISC), licences incluses |
| `vendor/egui_commonmark_backend` | petit patch : vrai gras / italique dans le mode lecture |

## Raccourcis

| | |
|---|---|
| Ctrl+P | Palette de commandes |
| Ctrl+O | Ouvrir / créer une note |
| Ctrl+N | Nouvelle note |
| Ctrl+E | Basculer édition / lecture |
| Ctrl+S | Enregistrer maintenant |
| Ctrl+Shift+F | Recherche |
| Ctrl+G | Graphe |
| F2 | Renommer / déplacer |
| Ctrl+T / Ctrl+W | Nouvel onglet / fermer l'onglet |
| Ctrl+clic | Ouvrir dans un nouvel onglet (explorateur) / suivre un lien (éditeur) |
| Ctrl+Shift+D | Tableau de bord |
| Ctrl+, | Paramètres |
