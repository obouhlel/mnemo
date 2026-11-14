# Audit de Mnemo — novembre 2026

Revue complète du code (bureau Rust, appli Android, paquets) et tests d'écriture simultanée sur une vraie base
PostgreSQL 16, avec deux instances de Mnemo pilotées au clavier sur deux écrans virtuels.

Les défauts trouvés ont été corrigés dans la version 0.5.0 ; ce document garde la trace de ce qui a été trouvé,
de ce qui a été vérifié, et de ce qui reste.

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

Même ligne modifiée des deux côtés : un bandeau « Conflit à résoudre » apparaît, la frappe continue, et la fenêtre
de résolution ne s'ouvre qu'à la demande avec le texte du moment.

## 2. Vérifications

| Quoi | Résultat |
|---|---|
| `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` | propres |
| `cargo test` avec `MNEMO_TEST_PG` (PostgreSQL 16) | 73 tests verts, dont 7 à deux appareils et un GitHub de bout en bout (faux serveur) |
| Android : `./gradlew testDebugUnitTest lintDebug` avec `MNEMO_TEST_PG` | 46 tests verts, lint propre |
| Deux instances à l'écran (Xvfb), frappe simultanée | scénarios du tableau ci-dessus |
| `schema.sql` | rejoué deux fois sur base vide, migration d'une base 0.4.0 avec données, ancien schéma rejoué par-dessus |
| `.deb` 0.4.0 publié | installé et lancé sous Ubuntu 24.04 |

**Non vérifié** : aucun essai sur un vrai Windows, macOS, Arch ou téléphone (ni émulateur). Les corrections
propres à ces systèmes (adresses `file:///C:/…`, noms réservés, chiffrement Keystore, clavier Android 15,
vérification TLS du téléphone, tâche `arch` de la CI) sont compilées et testées en unitaire seulement.

## 3. Bureau — corrigé

**Synchronisation** (`db/vault_sync.rs`, `local/mod.rs`) : fusion à 3 voies ; mises à jour conditionnées à la
version ; état connu avancé après le `COMMIT` et gardé dans `.mnemo/sync.json` + `.mnemo/base/` ; copies de
conflit jamais écrasées ; fichier relu avant chaque enregistrement ; chemins `..`, absolus ou `\` venus de la base
ignorés ; une note au nom refusé par le bureau n'est plus supprimée de la base.

**Éditeur et onglets** : enregistrement en échec réessayé ; texte d'un conflit non résolu ou refusé gardé dans
une note « (conflit date) » ; curseur stable quand du texte arrive ; conflit signalé par un bandeau sans
interrompre la frappe ; onglets et historique nettoyés quand une note est supprimée ; raccourcis inactifs derrière
les fenêtres ; image collée seulement si l'éditeur a le focus et dans la note d'origine ; fichier déposé sur
l'éditeur intégré à la note.

**Whiteboard** : fichier illisible ouvert en lecture seule (plus écrasé) ; éléments inconnus conservés ; fusion
avec les traits d'un autre appareil avant chaque enregistrement ; plus de plantage à la gomme ; onglets suivis au
déplacement d'un dossier ; texte en cours et dernier trait enregistrés à la fermeture ; annuler / rétablir cohérents.

**Liens et tags** (`local/index.rs`, `markdown.rs`) : le renommage ne réécrit que les liens qui désignaient la
note renommée, suit `[[école]]` et `[[École.md]]`, ne touche pas le code ; rétroliens résolus comme au clic ;
tags et liens ignorés dans le code ; `#123` n'est pas un tag ; ```` ```code``` ```` sur une ligne ne masque plus
la suite.

**Noms et configuration** : nouveaux noms refusés s'ils sont impossibles sous Windows ou cachés ; `config.toml`
invalide gardé à côté, écrit en 0600 ; `DATABASE_URL` ignoré (seul `MNEMO_DATABASE_URL` compte) ; jeton de
l'environnement jamais recopié ; `sslmode=verify-full` accepté ; message d'aide quand une bibliothèque
graphique manque ; images affichées sous Windows.

**GitHub et tickets** : lien GitHub effacé quand un ticket change de dépôt ; fusion champ par champ avant chaque
envoi (une issue fermée sur GitHub n'est plus rouverte) ; déplacement de ticket entre projets ; plus de limite à
1000 issues ; tags intégrés conservés à l'aller-retour ; plus d'issue en double ; titre et description d'un
ticket enregistrés pendant la frappe.

**Thèmes, plugins, schéma** : `var()` cyclique et `calc()` profond bornés ; `@media print` ignoré ; chaînes CSS
avec `\"` ; format de date invalide d'un plugin sans plantage ; bac à sable Rhai durci (tailles, `import`
désactivé, `main` dans le dossier) ; contraintes sur les chemins ; colonne `search` neutralisée ; schéma rejoué
sans verrou de table.

## 4. Android — corrigé

État local appliqué après le `COMMIT` ; changement d'adresse sans effacer les notes non envoyées ; enregistrement
à la sortie de la note ; une modification du bureau n'écrase plus un texte en cours (copie « conflit mobile ») ;
envoi et enregistrement simultanés sans perte ; whiteboard illisible protégé ; copies de conflit uniques ;
renommage en double ou vide refusé, une erreur sur une note ne bloque plus la synchro ; mêmes règles de noms que
le bureau ; images réduites et décodées hors du fil principal ; modification annulée sans faux conflit ; marges
clavier ; `verify-full` réellement vérifié ; adresse chiffrée (Keystore) et exclue des sauvegardes ; chemins
`../` ignorés ; `+` dans le mot de passe ; plus de note créée en touchant `![[doc.pdf]]` ; limite de 50 Mo.

## 5. Reste à faire

1. Sans `sslmode`, la connexion à la base n'est toujours pas chiffrée, sur le bureau comme sur le téléphone
   (le changer casserait les serveurs à certificat auto-signé) ; les réglages le disent maintenant.
2. Le mot de passe de la base et le jeton GitHub restent en clair dans `config.toml` (lisible par
   l'utilisateur seul).
3. Whiteboard : une écriture d'un autre appareil qui tombe dans les quelques millisecondes entre la
   vérification et l'écriture peut encore être perdue (`Cmd::WriteFile` n'a pas de contrôle de version).
4. Quitter une note avec un conflit non résolu crée une note « (conflit date) » à chaque fois.
5. Android : images au milieu d'une ligne et SVG non affichés ; pas de fusion (copie de conflit seulement) ;
   la position du curseur n'est pas gardée à la rotation.
6. Une ancienne version du bureau branchée sur la même base écrase encore les issues GitHub à sa façon :
   mettre tous les postes à jour.
7. Tri des noms accentués après « z » sur Android ; `lower()` de PostgreSQL dépend de la collation de la base.
8. macOS non notarisé et Windows non signé : avertissement au premier lancement (voir le README).

## 6. Paquets

- v0.4.0 publiée : `.deb`, `.tar.gz` x86_64 et aarch64, binaire Linux seul, `.dmg`, `.exe`. Ni APK, ni paquet
  Arch, ni les corrections ci-dessus.
- La release suivante (Actions › build › Run workflow, version `0.5.0`) ajoute `mnemo-android.apk` et
  `mnemo-0.5.0-1-x86_64.pkg.tar.zst` (tâche `arch`, jamais encore exécutée).
