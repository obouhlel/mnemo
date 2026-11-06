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

```bash
docker compose up -d          # PostgreSQL 16 sur localhost:5432 (base "vault")
cargo run --release
```

Au premier lancement, entrez l'URL de connexion (par défaut `postgres://postgres:postgres@localhost:5432/vault`).
Le schéma est créé automatiquement. Pour une base distante (Neon, Supabase, RDS…), ajoutez `?sslmode=require`.

Variables d'environnement utiles :

| Variable | Rôle |
|---|---|
| `MNEMO_DATABASE_URL` | URL Postgres (prioritaire sur la config) |
| `MNEMO_CONFIG_DIR` | dossier de config (par défaut le dossier de config de l'OS) |
| `MNEMO_PLUGINS_DIR` | dossier des plugins (par défaut `<config>/plugins`) |

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

## Thèmes et snippets CSS (format Obsidian)

Mnemo lit les thèmes et snippets CSS au format Obsidian et applique leurs **variables CSS** à l'interface native.
Paramètres › Apparence :

- **Dossier `.obsidian` d'un coffre existant** (facultatif) : ses `themes/<Nom>/theme.css` (ou `themes/<Nom>.css`),
  `snippets/*.css` et son `appearance.json` (thème choisi, snippets activés, `baseFontSize`, `accentColor`,
  clair/sombre) sont repris tels quels.
- Dossiers propres à Mnemo, même structure : `<config>/themes/` et `<config>/snippets/`
  (un snippet d'exemple `exemple-accent-vert.css` y est installé).
- Les fichiers sont **rechargés à chaud** : modifiez un `.css`, l'interface change dans la seconde.

Le moteur comprend `var()` (avec valeur de repli), `calc()`, `hsl()/hsla()`, `rgb()/rgba()`, hex, `@media`/`@supports`,
et les déclarations sur `body`, `:root`, `.theme-dark`, `.theme-light`. La chaîne de variables par défaut est la même que
dans Obsidian (`--color-base-00…100` → `--background-primary`…, `--accent-h/s/l` → `--interactive-accent`), donc un thème
qui ne change que quelques variables de base fonctionne.

Variables prises en charge : `--background-primary`, `--background-primary-alt`, `--background-secondary`,
`--background-modifier-border`, `--background-modifier-hover`, `--nav-item-background-hover/active`, `--text-normal`,
`--text-muted`, `--text-faint`, `--interactive-accent`, `--color-accent`, `--accent-h/s/l`, `--text-accent`, `--link-color`,
`--code-background`, `--code-normal`, `--tag-background`, `--tag-color`, `--checkbox-color`, `--blockquote-border-color`,
`--bold-color`, `--italic-color`, `--h1-color`…`--h6-color`, `--h1-size`…`--h6-size`, `--font-text-size`,
`--file-line-width`, `--line-height-normal`, `--text-selection`, `--color-base-*`, `--mono-rgb-100`.

Limites (l'interface n'est pas une page web) : les règles qui ciblent des éléments précis (`.nav-file-title { … }`),
`color-mix()` et les polices personnalisées ne sont pas appliquées ; le rapport des paramètres les signale.

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

## Écrire un plugin

Un plugin est un dossier dans le dossier des plugins (visible dans ⚙ Paramètres › Plugins) :

```
mon-plugin/
├── plugin.toml
└── main.rhai
```

```toml
id = "mon-plugin"
name = "Mon plugin"
version = "1.0.0"
description = "Ce que fait le plugin"

[[settings]]          # optionnel : réglages éditables dans l'interface
key = "prefixe"
label = "Préfixe"
default = "TODO"      # texte, nombre ou booléen ; kind = "multiline" pour un champ multiligne
```

Tous les hooks sont optionnels (`note` est une map `#{ id, path, title, content }`, ou `()` si aucune note n'est ouverte) :

```rhai
fn on_load() {}                              // à l'activation
fn commands() { [#{ id: "go", name: "Ma commande" }] }   // apparaissent dans Ctrl+P
fn run(command_id, note) { insert_text("Bonjour"); }
fn status(note) { "texte de la barre d'état" }
fn panel(note) { "## Markdown affiché à droite" }
fn preprocess(markdown, note) { markdown }   // transforme l'aperçu (pas la note)
fn on_open(note) {}
fn on_save(note) {}
fn on_event(event) {}                        // #{ kind, op, path } : événements live de la base
```

API disponible : `notice(msg)`, `log(msg)`, `open_note(path)`, `create_note(path, contenu)` (ouvre si elle existe),
`insert_text(t)`, `replace_content(t)`, `setting(clé)`, `note_paths()`, `date(fmt)`, `date_offset(jours, fmt)`,
`word_count(t)`, `char_count(t)`, `state_get(clé)`, `state_set(clé, valeur)`.

Plugins fournis (copiés au premier lancement, désactivés par défaut) : **Compteur de mots**, **Notes quotidiennes**,
**Plan** (table des matières), **Encadrés** (`> [!note]`, `> [!warning]`…), **Modèles**.
Les scripts sont limités en nombre d'opérations : un plugin bogué ne peut pas figer l'application.

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
