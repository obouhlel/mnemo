# Mnemo

Base de connaissances Markdown **native** (Rust + egui, sans Electron, sans JS). Les notes sont de simples
fichiers `.md` dans un dossier (un coffre Obsidian convient) : Mnemo fonctionne **sans base de données**.
Une base **PostgreSQL** facultative les **synchronise en direct** entre vos appareils, et tout continue en local
hors connexion. Un seul binaire pour Windows, macOS et Linux.

## Fonctionnalités

- Éditeur Markdown avec **aperçu en direct** (modes Édition / Divisé / Lecture), coloration du code, cases à cocher cliquables
- `[[Liens wiki]]` avec **autocomplétion** (`[[` + nom), `[[Note|alias]]`, `[[Note#titre]]`, création au clic si la note n'existe pas
- **Rétroliens**, `#tags`, recherche plein texte (sans accent ni casse, extraits surlignés), arborescence de dossiers
- Renommer / déplacer une note **met à jour les liens** dans les autres notes ; **glisser-déposer** des notes et dossiers
  dans l'explorateur, et import de fichiers `.md` / dossiers déposés depuis le système
- **Images** : `![[photo.png]]` (comme Obsidian) ou `![](dossier/photo.png)`, affichées dans l'éditeur et en lecture ;
  une capture collée (Ctrl+V) ou une image déposée sur la note est rangée dans le dossier des pièces jointes (réglage
  d'Obsidian, sinon « Pièces jointes ») et intégrée au curseur ; pièces jointes visibles dans l'explorateur et
  synchronisées entre appareils (jusqu'à 50 Mo par fichier)
- **Whiteboard** à dessin libre (icône du ruban, palette ou clic droit sur un dossier) : stylo, ligne, flèche, rectangle,
  ellipse, texte, gomme, sélection et déplacement, 6 couleurs, 3 épaisseurs, annuler / rétablir, zoom (Ctrl+molette) ;
  fichier `.mnemo-board` (JSON) enregistré automatiquement et synchronisé, intégrable dans une note avec
  `![[croquis.mnemo-board]]` (aperçu cliquable dans l'éditeur)
- **Icônes** de dossiers et de notes dans l'explorateur (clic droit › Changer l'icône…) : les 1 500 icônes Lucide ou
  un emoji, synchronisées entre appareils ; celles du plugin Iconize d'Obsidian sont reprises à la liaison du coffre
- **Dossier des notes** (Paramètres › Dossier des notes) : un dossier de fichiers `.md`, par exemple un coffre Obsidian ;
  un fichier modifié par un autre éditeur apparaît dans Mnemo en une seconde ; suppressions dans `.trash/`
- **Synchronisation facultative** (Paramètres › Synchronisation) : avec une base PostgreSQL, chaque modification
  (depuis une autre machine ou même un `UPDATE` SQL) apparaît en direct ; hors ligne, tout est gardé en local et envoyé
  au retour de la connexion
- **Résolution de conflits façon GitHub** : fusion à 3 voies, les changements qui ne se chevauchent pas sont fusionnés automatiquement ; les autres s'affichent côte à côte avec un diff unifié et un choix par bloc
- **Plugins** en [Rhai](https://rhai.rs) (scripting embarqué en Rust, sandboxé), activables et configurables, réglages synchronisés
- Palette de commandes (Ctrl+P), sélecteur de notes (Ctrl+O), graphe des liens (Ctrl+G, optionnel), thème clair/sombre
- Interface calquée sur le thème par défaut d'Obsidian : ruban, explorateur avec chevrons et guides, onglets avec historique
  (← →), fil d'Ariane, titre intégré éditable (renomme la note), colonne de lecture de 700 px ; aide, paramètres et état de la
  synchronisation en bas du ruban
- **Live preview** dans l'éditeur : la syntaxe markdown (`**`, `##`, `[[ ]]`…) n'apparaît que sur la ligne du curseur ;
  puces, cases à cocher cliquables, citations et blocs de code dessinés ; Ctrl+clic sur un lien pour l'ouvrir

## Démarrage rapide

1. Lancez Mnemo (`cargo run --release`, ou le fichier de la page [Releases](https://github.com/obouhlel/mnemo/releases/latest)).
   Les notes sont rangées dans `~/Documents/Mnemo` ; choisissez un autre dossier (par exemple votre coffre Obsidian)
   dans Paramètres › Dossier des notes.
2. Facultatif, pour synchroniser plusieurs appareils : démarrez une base PostgreSQL (ou utilisez Neon, Supabase, RDS…)
   ```bash
   make db                       # PostgreSQL 16 dans Docker, sur localhost:5432 (base "vault")
   ```
   puis entrez son URL dans Paramètres › Synchronisation (`postgres://postgres:postgres@localhost:5432/vault`,
   `?sslmode=require` pour une base distante). Le schéma est créé automatiquement et vos notes y sont envoyées.
3. Sur un autre ordinateur, entrez la même URL : ses notes arrivent dans son dossier, puis tout est synchronisé en direct.
   Le tableau de bord (projets, tickets, GitHub) est partagé par la base : il demande la synchronisation.

Sous Linux, si une bibliothèque graphique manque, Mnemo affiche la commande `apt` / `pacman` / `dnf` à lancer.

Variables d'environnement utiles :

| Variable | Rôle |
|---|---|
| `MNEMO_DATABASE_URL` | URL de la base de synchronisation (prioritaire sur la config) |
| `MNEMO_CONFIG_DIR` | dossier de config (par défaut le dossier de config de l'OS) |
| `MNEMO_PLUGINS_DIR` | dossier des plugins (par défaut `<config>/plugins`) |

## Documentation

| Document | Contenu |
|---|---|
| [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) | organisation du code, flux de synchronisation, recettes (ajouter une commande…) |
| [docs/plugins.md](docs/plugins.md) | écrire un plugin Rhai : `plugin.toml`, hooks, API |
| [docs/themes.md](docs/themes.md) | thèmes et snippets CSS au format Obsidian |
| [docs/RAPPORT_TESTS.md](docs/RAPPORT_TESTS.md) | ce qui a été testé et comment |

## Installation

Téléchargez le fichier de votre système sur la page [Releases](https://github.com/obouhlel/mnemo/releases/latest).
Linux (Arch, Fedora, Debian…), sans rien installer d'autre :

```bash
curl -LO https://github.com/obouhlel/mnemo/releases/latest/download/mnemo-linux-x86_64 && chmod +x mnemo-linux-x86_64 && ./mnemo-linux-x86_64
```

| OS | Installer |
|---|---|
| **Debian / Ubuntu** | `sudo apt install ./mnemo_<version>-1_amd64.deb` |
| **Arch Linux** | `make arch` puis `sudo pacman -U dist/mnemo-*.pkg.tar.zst` |
| **Autres Linux** | archive `mnemo-<version>-linux-x86_64.tar.gz` / `aarch64` (glibc ≥ 2.28) |
| **Windows 10/11** | `mnemo.exe` |
| **macOS 11+** (Apple Silicon et Intel) | `Mnemo.app` / `.dmg` |

Le binaire Linux tourne sur Debian 10+, Ubuntu 18.04+, Arch, Fedora… Bibliothèques nécessaires (présentes sur tout
poste de bureau) :

- Debian / Ubuntu : `sudo apt install libx11-6 libxkbcommon-x11-0 libgl1 libegl1 libwayland-client0 libxcursor1 libxrandr2 libxi6`
- Arch : `sudo pacman -S libx11 libxkbcommon-x11 libglvnd mesa wayland libxcursor libxrandr libxi`

## Compiler

Tout passe par le `Makefile` (`make` seul liste les cibles). Les binaires et paquets sont écrits dans `dist/`.

| Commande | Résultat |
|---|---|
| `make run` | base Docker + Mnemo en release |
| `make check` | formatage, clippy et tests (à lancer avant chaque commit) |
| `make setup` | une fois : cibles Rust croisées, `cargo-zigbuild`, `cargo-deb` (nécessite rustup, zig, mingw-w64) |
| `make linux` | `dist/mnemo-<version>-linux-{x86_64,aarch64}.tar.gz` (glibc 2.28, via `cargo zigbuild`) |
| `make deb` | `dist/*.deb` |
| `make arch` | `dist/*.pkg.tar.zst` |
| `make windows` | `dist/mnemo.exe` (MinGW depuis Linux, ou natif sous Windows) |
| `make macos` | `dist/Mnemo.app` + `.dmg`, **uniquement sur un Mac** (SDK Apple) |
| `make all` | tout ce que la machine peut compiler (`linux deb windows`, ou `macos` sur un Mac) |
| `make clean` | supprime `target/` et `dist/` |

Le workflow `.github/workflows/release.yml` appelle les mêmes cibles sur des machines natives, y compris macOS
(Actions › Run workflow, ou en poussant un tag `v*`). Pour publier une version sans git : Actions › build ›
Run workflow avec le champ « version » (ex. `0.2.0`) ; la page Releases est créée avec tous les fichiers.
Le workflow `.github/workflows/macos-smoke.yml` (lancé à la main) compile `Mnemo.app`, l'ouvre sur un vrai Mac avec
PostgreSQL, vérifie qu'elle se connecte et joint une capture d'écran en artefact.
`.github/workflows/windows-smoke.yml` fait de même sous Windows avec le `mnemo.exe` compilé comme pour une livraison.

## Tableau de bord : projets, tickets et GitHub

Ouvrez-le avec l'icône du ruban ou **Ctrl+Shift+D**. Tout est stocké dans PostgreSQL et synchronisé en direct.

- **Projets** contenant des **tickets** numérotés (T-1, T-2…), avec statut (À faire, En cours, Bloqué, En revue, Terminé),
  priorité (Basse → Urgente), description Markdown et **tags** : Feature, Bug, Amélioration, Documentation,
  Tâche technique, Sécurité, Performance, Recherche, vos propres tags, ou les labels du dépôt GitHub lié.
- **Vue « Lignes »** : todo list par projet ; chaque projet se déplie en lignes de tags, qui se déplient en tickets
  (case à cocher pour terminer, ajout rapide d'un ticket dans chaque tag). Un ticket avec plusieurs tags apparaît dans chacun.
- **Vue « Colonnes »** : kanban par statut, glisser-déposer des cartes.
- **Avancement** : barres de progression globale, par projet et par tag ; compteurs par statut.
- **Rapports** : des notes liées aux tickets. « Nouveau rapport » crée une note pré-remplie dans `Rapports/<Projet>/`,
  ou liez une note existante ; dans une note, l'onglet *Tickets liés* du panneau de droite montre et gère ses tickets.
- **GitHub** (Paramètres › GitHub : token personnel, URL d'API pour GitHub Enterprise) puis, par projet, un dépôt
  choisi dans la liste de vos dépôts (ou saisi en `propriétaire/nom`) :
  - À l'enregistrement, toutes les **issues et pull requests** existantes (ouvertes, fermées, fusionnées) sont
    importées. Une PR ouverte est « En revue », un brouillon « En cours », une PR fusionnée ou fermée « Terminé ».
  - Les **tags sont les labels du dépôt**, avec le même nom et la même couleur ; le choix des tags d'un ticket
    propose ces labels.
  - **Synchroniser** applique les changements des deux côtés : titre, description, labels, ouverte/fermée ↔ statut.
    Le côté modifié depuis la dernière synchro gagne ; si les deux ont changé, le plus récent. Une pull request n'est
    jamais fermée depuis Mnemo : seuls son titre, sa description et ses labels sont envoyés.
  - **Publier sur GitHub** crée l'issue d'un ticket local ; ensuite chaque modification (statut, titre, tags…) est
    poussée automatiquement. Synchronisation automatique toutes les 5 minutes (désactivable).
  - Le token reste dans le fichier de configuration local (ou `MNEMO_GITHUB_TOKEN`), jamais dans la base.

## Comment marche la synchronisation

```
   Appareil A                                                   Appareil B
 dossier .md ◄─► stockage local ◄─► synchro ◄─► PostgreSQL ◄─► synchro ◄─► stockage local ◄─► dossier .md
                  (src/local/)    (vault_sync)   NOTIFY ──────► (immédiat)
```

- **Le dossier est la référence.** Le stockage local (`src/local/`) indexe les fichiers `.md`, répond à l'interface
  (recherche, rétroliens, tags, graphe, renommage avec réécriture des `[[liens]]`) et voit en une seconde un fichier
  modifié par un autre éditeur. Icônes et réglages des plugins sont dans `.mnemo/` à côté des notes.
- **La base est facultative.** Si elle est configurée, le worker compare le dossier à la base toutes les 2 s (et tout de
  suite quand un autre appareil écrit, grâce à `LISTEN/NOTIFY`) et recopie les changements dans les deux sens :
  contenu, renommages, suppressions, dossiers, icônes, plugins. Une note garde le même identifiant en local et en base.
- **Hors ligne**, rien ne bloque : les modifications restent dans le dossier et le worker réessaie toutes les 30 s ;
  au retour, tout est recomparé et envoyé. Si une note a changé des deux côtés, la base l'emporte et le texte local
  est gardé dans une note `… (conflit disque)`.
- Dans l'éditeur, sauvegarde automatique 0,6 s après la dernière frappe ; si la note a changé ailleurs pendant que vous
  écriviez, fusion à 3 voies (`src/merge.rs`) : rien n'est écrasé sans votre accord.

## Structure

| Chemin | Rôle |
|---|---|
| `src/app.rs`, `src/app/` | interface egui (un fichier par zone : éditeur, barres latérales, tableau de bord…) |
| `src/local/` | stockage local : dossier de fichiers `.md`, index (recherche, liens, tags, graphe), `.mnemo/` |
| `src/db/` | worker Postgres : connexion et LISTEN, synchro dossier ↔ base, tableau de bord / GitHub |
| `src/schema.sql` | tables, triggers de version / liens / NOTIFY |
| `src/merge.rs`, `src/board.rs`, `src/github.rs` | fusion à 3 voies, domaine du tableau de bord, client GitHub |
| `src/plugins.rs`, `src/css.rs`, `src/theme.rs` | plugins Rhai, thèmes CSS, apparence |
| `assets/` | polices, icônes, plugins d'exemple, textes intégrés |
| `vendor/egui_commonmark_backend` | petit patch : vrai gras / italique dans le mode lecture |

Le détail de chaque module est dans [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

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
