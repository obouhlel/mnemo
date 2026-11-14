# Audit de Mnemo — novembre 2026

Revue complète du code (bureau Rust, appli Android, paquets) et tests d'écriture simultanée sur une vraie base
PostgreSQL 16, avec deux instances de Mnemo pilotées au clavier sur deux écrans virtuels.

« Corrigé » : corrigé dans cette branche, avec un test. Tout le reste est **à faire**. Sauf mention « reproduit »,
les constats viennent de la lecture du code, chemin suivi de bout en bout.

## 1. Écriture simultanée sur la même note

| Scénario | Avant | Après correction |
|---|---|---|
| A et B tapent en même temps sur deux lignes différentes | texte de B perdu, note « (conflit disque) » parasite, faux conflit affiché chez B (reproduit) | les deux textes sont là des deux côtés, aucun conflit |
| Frappe continue 12 s des deux côtés | idem | les deux phrases entières, dans l'ordre |
| A et B tapent sur la même ligne | fenêtre de conflit, mais une 2ᵉ copie de conflit était jetée | fenêtre de conflit, chaque texte gardé (copies numérotées) |
| Un autre éditeur écrit le fichier moins d'une seconde avant l'enregistrement de Mnemo | écrasé sans conflit (reproduit) | conflit signalé, fusion |
| B éteint ou hors ligne modifie la note, A la modifie ensuite, B revient | la modification de B écrasée sans trace (reproduit) | fusion à 3 voies |
| Texte distant qui arrive pendant la frappe | le curseur sautait, mots dans le désordre (reproduit) | le curseur reste sur sa ligne |

Capture avant correction (B a tapé sur la ligne 9, A sur la ligne 2) : [audit/avant-faux-conflit.png](audit/avant-faux-conflit.png).

Tests : `MNEMO_TEST_PG=postgres://… cargo test sync_tests` (7 tests, 6 échouent sur l'ancien code).

Limite restante : quand la fenêtre « Résoudre le conflit » s'ouvre pendant la frappe (même ligne modifiée des deux
côtés), les touches tapées ensuite ne vont plus dans la note.

## 2. Corrigé dans cette branche

- `db/vault_sync.rs` : fusion à 3 voies quand fichier et base ont changé ; `UPDATE` / `DELETE` seulement si la
  version en base est celle attendue ; l'état connu n'avance qu'après le `COMMIT` (avant : un passage interrompu
  pouvait réécrire un fichier avec l'ancien texte ou mettre une pièce jointe à la corbeille) ; état gardé dans
  `.mnemo/sync.json` et `.mnemo/base/` ; copies de conflit jamais écrasées ; à la première rencontre le texte
  écarté va dans `.trash/`.
- Chemins venus de la base (`..`, chemin absolu, `\`, `C:`) : ignorés au lieu d'être écrits hors du dossier des
  notes. Une note au nom refusé par le bureau (créée sur le téléphone, ex. « Réunion #3 ») n'est plus supprimée
  de la base.
- `local/mod.rs` : fichier relu avant chaque enregistrement, création et renommage.
- Éditeur : un enregistrement en échec est réessayé (avant : plus aucune sauvegarde de la note jusqu'au
  redémarrage) ; le texte d'un conflit non résolu, ou refusé après avoir quitté la note, est gardé dans une note
  « (conflit date) » ; idem si la note ouverte est supprimée ailleurs ; à la fermeture, le texte tapé pendant un
  enregistrement en cours est enregistré.

## 3. Bureau — à faire

### Perte de données
1. Whiteboard illisible (JSON invalide, type d'élément inconnu, fichier verrouillé) ouvert comme vide puis
   écrasé par l'enregistrement automatique — `app/whiteboard.rs:70-77, 306-320`.
2. Whiteboard : l'enregistrement écrase les traits ajoutés par un autre appareil pendant un tracé ; Ctrl+Z après
   un rechargement les efface — `app/whiteboard.rs:114-120, 315-331`.
3. Renommer une note réécrit les liens qui pointent vers une *autre* note de même nom (`Plan` et `Archive/Plan`),
   y compris au déplacement d'un dossier (reproduit) — `local/index.rs:240-250`, `db/notes.rs:400-420`.
4. Renommage : les liens dans les blocs de code sont réécrits (`if [[ old ]]` en shell) (reproduit) ;
   `[[école]]` et `[[École.md]]` ne suivent pas le renommage de `École` (reproduit) — mêmes fichiers.
5. Titre et description d'un ticket enregistrés seulement à la perte du focus : perdus avec Ctrl+W ou à la
   fermeture — `app/dashboard/ticket.rs:45-56, 139-143`.
6. Whiteboard ouvert dont le fichier ou le dossier est déplacé : l'onglet devient vide et recrée l'ancien
   chemin — `app/whiteboard.rs:326-336`.
7. `config.toml` invalide : remplacé par les valeurs par défaut au lancement (URL, jeton, dossier perdus) —
   `config.rs:80-109`.

### Plantages
8. Bibliothèque graphique manquante sous Linux : panique au lieu du message d'aide annoncé (reproduit avec
   `libxkbcommon-x11`) — `main.rs:38`.
9. Format de date invalide dans un réglage de plugin (`%Q`, `100%`) : plantage (reproduit) — `plugins.rs:196-199`.
10. Touche d'outil pendant un coup de gomme sur le whiteboard : index hors limites — `app/whiteboard.rs:466`.
11. Thème CSS : `var()` auto-référencé (gel de plus d'une minute, reproduit), `calc(((((…` (débordement de pile,
    reproduit) — `css.rs:174-304`.
12. `&path[from.len()..]` après comparaison en minuscules : découpe fausse ou panique sur `İ` —
    `local/mod.rs` (`rename_folder`), `local/shared.rs:62`.

### Sécurité
13. `DATABASE_URL` de l'environnement (autre projet, direnv) adoptée : Mnemo s'y connecte, y crée ses tables
    (`notes`, `projects`, `tickets`…) et enregistre l'URL — `config.rs:92`. Ne garder que `MNEMO_DATABASE_URL`.
14. Mot de passe de la base et jeton GitHub en clair dans `config.toml` ; `MNEMO_GITHUB_TOKEN` y est recopié ;
    `redact_url` ne masque pas la forme `host=… password=…` — `config.rs`.
15. Sans `sslmode`, connexion en clair même vers un serveur distant ; `sslmode=verify-full` refusé par le pilote
    — `db/connection.rs:35-37, 90`.
16. Plugins Rhai : pas de limite de taille des tableaux (mémoire), `import` de fichiers hors du dossier du
    plugin, `main = "../../x"` accepté — `plugins.rs:150-154, 238`.
17. Le `CHECK` du schéma accepte encore `..` et `\` dans `notes.path` / `files.path` (le bureau les ignore
    maintenant, le téléphone non) — `schema.sql:5, 21, 118`.

### GitHub et tableau de bord
18. Un ticket garde son numéro d'issue en changeant de projet ou de dépôt : la synchro écrase une issue sans
    rapport dans l'autre dépôt — `db/board.rs:35-87`, `db/github_sync.rs:64-85`.
19. Modifier un ticket rouvre et écrase une issue fermée ou modifiée entre-temps sur GitHub —
    `db/board.rs:88-104`.
20. Déplacer un ticket vers un autre projet échoue presque toujours (`tickets_number_uniq`) — `schema.sql:163`.
21. Import limité à 1000 issues sans avertissement ; les tags « Amélioration » / « Tâche technique » reviennent
    en `enhancement` / `chore` ; issue créée en double si la fermeture échoue après la création —
    `github.rs:125-144`, `board.rs:125, 290`, `db/github_sync.rs:97-113`.

### Autres
22. Images des notes jamais affichées sous Windows (`file://C:\…` au lieu de `file:///C:/…`) ; une image en
    échec redemande un affichage toutes les 100 ms — `app/attachments.rs:43, 145`, `app/editor.rs:117-120`.
23. Noms acceptés mais impossibles sous Windows (`: * ? " < >`, `CON`, point final) ; notes dont un dossier
    commence par `.` créées puis invisibles — `model.rs:87-104`.
24. Tags et liens lus dans les blocs de code : `#include`, `#fff`, `[[ -z "$x" ]]` (reproduit) —
    `local/index.rs:15-67`.
25. Onglets et historique gardent les notes supprimées avec leur dossier ; l'onglet affiche alors la note d'un
    autre onglet — `app/sync.rs`, `app/dialogs.rs:211-218`.
26. Raccourcis globaux actifs derrière les fenêtres (Paramètres, icônes) ; image collée insérée dans la note
    même quand le focus est dans la recherche ; `![[…]]` inséré dans la note ouverte à l'arrivée de la réponse —
    `app/commands.rs:268`, `app/attachments.rs:88`.
27. Fichier déposé sur l'éditeur parfois importé à la racine au lieu d'être intégré — `app/sidebar.rs:370`.
28. Une note de plus de ~1 Mo de mots distincts fait échouer toute la synchro (colonne `search` inutilisée) ;
    le schéma rejoué à chaque connexion verrouille toutes les tables — `schema.sql:11-17, 56-58`.
29. `@media print` d'un thème appliqué à l'écran ; `content: "\""` arrête la lecture du thème — `css.rs`.
30. Une ligne ```` ```code``` ```` masque les liens du reste de l'aperçu — `markdown.rs:68-73`.
31. `Msg::Renamed` change `last_note` même pour une note non ouverte — `app/sync.rs`.
32. `db/notes.rs` : commandes SQL des notes jamais appelées (tout passe par le stockage local), à supprimer.

## 4. Android — à faire

### Perte de données
1. L'état local est noté « envoyé » avant le `COMMIT` PostgreSQL : une coupure pendant la synchro fait écraser
   ou supprimer les modifications du téléphone au passage suivant — `SyncEngine.kt:64-81`.
2. Changer l'adresse de la base (même pour ajouter `?sslmode=require`) efface toutes les notes locales, y
   compris celles jamais envoyées — `AppModel.kt:174`, `LocalDb.kt:197-203`.
3. Enregistrement automatique de 800 ms sans enregistrement à la sortie : texte perdu en quittant la note juste
   après la frappe ou en tournant l'écran — `NoteScreen.kt:97-103`.
4. Une modification du bureau arrivée pendant la frappe est écrasée sans copie de conflit —
   `NoteScreen.kt:88-103`, `SyncEngine.kt:147-155`.
5. Enregistrement pendant un envoi : la note (ou le trait de whiteboard) reste « envoyée » avec l'ancien
   contenu — `SyncEngine.kt:97-126, 210-223`.
6. Whiteboard illisible ouvert comme vide puis envoyé à la place du dessin — `BoardScreen.kt:176-196`.
7. Copie de conflit perdue si deux conflits dans la même minute (`ON CONFLICT DO NOTHING`) —
   `SyncEngine.kt:85-86, 130`.

### Blocages et plantages
8. Renommer vers un nom déjà pris ou vide bloque toute la synchro (erreur SQL toutes les 15 s) —
   `SyncEngine.kt:119-124`, `AppModel.kt:211-217`.
9. Deux notes de même chemin : plantage de la liste à chaque lancement (clé dupliquée) —
   `NotesListScreen.kt:145`.
10. Noms `[ ] | # ^ \` et `..` acceptés alors que le bureau les refuse — `AppModel.kt:201-217`.
11. Images décodées en pleine taille sur le fil principal, toutes les notes relues à chaque enregistrement,
    pièce jointe entière en mémoire : lenteurs et manque de mémoire — `NoteScreen.kt:246`, `AppModel.kt:94-99`,
    `SyncEngine.kt:237-243`.
12. Modification annulée (taper puis effacer) vue comme un conflit — `SyncEngine.kt:119-133`.
13. Clavier probablement par-dessus l'éditeur sous Android 15 (`targetSdk 35`, pas d'`imePadding`).

### Sécurité
14. TLS jamais vérifié, même avec `sslmode=verify-full` — `Util.kt:38-40`.
15. Adresse de la base avec mot de passe en clair dans les préférences et dans les sauvegardes Android
    (`allowBackup="true"`).
16. Chemin de pièce jointe `../…` venu de la base écrit hors du dossier — `LocalDb.kt:170`.
17. `+` dans le mot de passe transformé en espace — `Util.kt:30-31`.

### Divers
18. `![[doc.pdf]]` ou image manquante : un appui crée une note « doc.pdf » ; `.svg` non reconnu ; images en
    ligne non affichées — `Preprocess.kt`.
19. Pièces jointes de 20 à 50 Mo listées mais jamais téléchargées ; `lower()` de SQLite limité à l'ASCII
    (« École » / « école » en double).

## 5. Paquets

- La version publiée (v0.4.0) contient `.deb`, `.tar.gz` x86_64 et aarch64, binaire Linux seul, `.dmg`, `.exe`.
  Le `.deb` a été installé et lancé ici sous Ubuntu 24.04 : il fonctionne.
- Pas d'APK ni de paquet Arch dans v0.4.0 (le README annonce `mnemo-android.apk`) : il faut publier une
  nouvelle version ; ajouter `make arch` au workflow `release.yml` (conteneur `archlinux`) fournirait le
  `.pkg.tar.zst`.
- macOS : signature ad hoc, pas de notarisation ; sous macOS 15 le clic droit › Ouvrir ne suffit plus (voir
  l'installation). Windows : exécutable non signé (SmartScreen), pas d'installeur.
- `.deb` pour amd64 seulement.
