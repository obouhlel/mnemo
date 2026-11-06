# Rapport de tests — Mnemo 0.1.0

Tous les tests ont été faits sur le binaire **release**, avec une base PostgreSQL 16 vierge, une configuration vierge
et un vrai affichage (serveur X virtuel), en pilotant l'interface au clavier et à la souris. Chaque résultat a été
vérifié dans la base de données et sur capture d'écran.

## Tests automatiques

`cargo test` : **19 tests, tous verts** — wikilinks/tags, réécriture des liens au renommage, fusion à 3 voies et diff,
exécution des 5 plugins Rhai, lecteur CSS (var(), calc(), hsl…), coloration de l'éditeur, plan de synchronisation GitHub,
correspondance labels ↔ tags, analyse des réponses GitHub, gestion de NO_PROXY. `cargo clippy` : aucun avertissement.

## Scénarios de bout en bout

| # | Scénario | Résultat |
|---|---|---|
| 1 | URL de base invalide | message d'erreur clair, pas de plantage |
| 2 | Premier lancement sur base vierge | schéma (4+3 tables, triggers) créé, note de bienvenue, plugins et snippet installés |
| 3 | Nouvelle note (Ctrl+N) + saisie + autocomplétion `[[` | note enregistrée, lien inséré, table des liens à jour |
| 4 | Deux instances côte à côte | la frappe dans A apparaît dans B (« mis à jour à distance ») |
| 5 | Conflit (même ligne modifiée des deux côtés) | fenêtre de résolution, choix « les deux », résultat correct en base |
| 6 | Renommage via le titre | chemin changé, 3 formes de liens réécrites dans les autres notes |
| 7 | Plugins activés depuis l'interface | état en base, note du jour, compteur de mots, plan |
| 8 | Onglets / historique / sélecteur rapide | ← → , Ctrl+Entrée (nouvel onglet), Ctrl+W |
| 9 | Ctrl+clic sur un lien dans l'éditeur | ouvre la note (avec et sans focus) ; clic simple n'ouvre pas |
| 10 | Recherche plein texte et tags | résultat surligné, focus automatique du champ, liste des tags |
| 11 | Suppression avec confirmation | note supprimée, onglet vidé |
| 12 | Redémarrage de PostgreSQL pendant l'utilisation | reconnexion automatique, modification distante reçue |
| 13 | Mode lecture + plugin Encadrés | rendu correct |
| 14 | Thème CSS Obsidian (coffre de test, palette Nord) + snippet | variables appliquées, rechargement à chaud < 2 s |
| 15 | Tableau de bord : création de projet relié à GitHub | URL normalisée en `propriétaire/dépôt` |
| 16 | Synchronisation GitHub (faux serveur GitHub fidèle à l'API) | 3 issues importées, pull request ignorée, labels → tags |
| 17 | Case à cocher d'un ticket lié | issue fermée sur GitHub |
| 18 | Publication d'un ticket | issue créée avec titre, description, labels |
| 19 | Modification côté GitHub + synchro | titre, état et tags mis à jour dans Mnemo |
| 20 | Nouveau rapport depuis un ticket | note pré-remplie, liée, ouverte ; onglet « Tickets liés » |
| 21 | Kanban glisser-déposer | statut changé en base |
| 22 | Modification des tickets en SQL (autre machine) | tableau de bord mis à jour en direct |

## Compatibilité des systèmes

| Système | Test | Résultat |
|---|---|---|
| Ubuntu 24.04 | tous les scénarios ci-dessus | ✔ |
| Ubuntu 22.04 (Docker) | lancement graphique + création d'une note en base | ✔ |
| Ubuntu 20.04 (Docker) | lancement graphique + création d'une note en base | ✔ |
| Debian 10 (glibc 2.28) | chargement du binaire (glibc) | ✔ — test graphique impossible ici : miroirs Debian bloqués par le réseau de l'environnement |
| Debian 12 | chargement du binaire | ✔ (même limite) |
| Arch Linux | chargement du binaire | ✔ (même limite) |
| Windows | compilation (`mnemo.exe`, icône intégrée) | compilé, **non exécuté** (pas de Windows ici) |
| macOS | — | **non compilé ici** (nécessite le SDK Apple) : script `build-macos.sh` + GitHub Actions fournis |

Le binaire Linux ne demande que glibc 2.28 ; X11/Wayland/OpenGL sont chargés au lancement.

## Bugs trouvés par ces tests et corrigés

1. Titre affiché en double sur les nouvelles notes (le titre intégré suffit) — notes créées vides.
2. Renommage par le titre envoyé deux fois (erreur « unexpected number of rows ») et ligne vide insérée.
3. Ctrl+clic sur un lien inopérant quand l'éditeur n'avait pas le focus.
4. Le champ de recherche ne recevait pas le focus à l'ouverture.
5. Modèle de note du jour avec titre en double ; espacement de la page Plugins.
6. Identifiants d'interface en conflit pour un ticket affiché dans plusieurs tags.
7. Barre d'outils du tableau de bord qui se chevauchait sur fenêtre moyenne.
8. Noms longs sur deux lignes dans l'arborescence, titre intégré coupé, cartes kanban justifiées.
9. NO_PROXY ignoré par le client HTTP (GitHub Enterprise derrière un proxy).
