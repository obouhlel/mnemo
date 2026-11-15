# Thèmes et snippets CSS (format Obsidian)

Mnemo lit les thèmes et snippets CSS au format Obsidian et applique leurs **variables CSS** à l'interface native.

## Paramètres › Apparence

- **Mode** : Clair, Sombre ou Système (suit le réglage clair / sombre du système d'exploitation).
- **Thème** : « Par défaut » ou l'un des thèmes installés.
- **Couleur d'accent** : liens, cases cochées, sélection, éléments actifs. Le bouton ↺ revient à celle du thème.
- **Police de l'interface**, **police du texte** (éditeur, mode lecture, titre de la note) et **police du code** :
  « Par défaut » ou l'une des polices installées sur le système. « Par défaut » garde la police du thème
  (`--font-interface-theme`, `--font-text-theme`, `--font-monospace-theme`), sinon celle de Mnemo (Inter).
- **Taille du texte** des notes, en pixels.
- **Snippets CSS** : un interrupteur par snippet ; le chemin du fichier s'affiche en infobulle. Les deux icônes à
  côté du titre rechargent les fichiers (l'infobulle liste les dossiers lus) et ouvrent le dossier des snippets.
- **Rapport de chargement**, en bas de la page : ce qui a été lu et appliqué (thème, snippets, variables, polices).

### Utiliser l'apparence du coffre Obsidian

Quand le dossier des notes est un coffre Obsidian (il contient un dossier `.obsidian`), Mnemo propose l'interrupteur
**Utiliser l'apparence du coffre Obsidian**. Activé, il reprend tels quels les réglages du coffre :

- `.obsidian/appearance.json` : thème choisi (`cssTheme`), snippets activés (`enabledCssSnippets`), taille du texte
  (`baseFontSize`), couleur d'accent (`accentColor`), mode clair / sombre (`theme` : `obsidian` = sombre,
  `moonstone` = clair), polices (`textFontFamily`, `interfaceFontFamily`, `monospaceFontFamily`) et ruban
  (`showRibbon`) ;
- `.obsidian/app.json` : titre intégré (`showInlineTitle`) et largeur de lecture (`readableLineLength`).

Ces valeurs sont alors affichées en lecture seule ; le lien **Personnaliser** désactive l'interrupteur en gardant les
valeurs du coffre comme point de départ.

Si le dossier des notes n'est pas un coffre, la section **Avancé** permet de choisir un autre coffre Obsidian
(bouton « Parcourir… ») dont reprendre l'apparence. Ce réglage s'appelait « Suivre appearance.json » dans les versions
précédentes ; les anciennes configurations continuent de fonctionner.

## Où sont lus les fichiers

- Dossier `.obsidian` du coffre : `themes/<Nom>/theme.css` (ou `themes/<Nom>.css`) et `snippets/*.css`.
- Dossiers propres à Mnemo, même structure : `<config>/themes/` et `<config>/snippets/`
  (un snippet d'exemple `exemple-accent-vert.css` y est installé).
- Les fichiers sont **rechargés à chaud** : modifiez un `.css`, l'interface change dans la seconde.

Les polices sont cherchées parmi celles installées sur le système (nom exact de la famille). La police du texte
s'applique à l'éditeur, à l'aperçu et au titre de la note ; l'interface garde la sienne, comme dans Obsidian.
Les emoji sont dessinés en couleur si une police d'emoji couleur est installée (Noto Color Emoji, Segoe UI Emoji…).

## Variables CSS

Le moteur comprend `var()` (avec valeur de repli), `calc()`, `hsl()/hsla()`, `rgb()/rgba()`, `color-mix()` (mélangé en
sRGB), hex, `@media`/`@supports`, et les déclarations sur `body`, `:root`, `.theme-dark`, `.theme-light`.
Les règles qui visent les éléments du texte sont traduites en variables : titres (`.markdown-rendered h1`,
`.cm-header-1`… : `color`, `font-size`), code en ligne (`code`, `.cm-inline-code`), citations (`blockquote` :
`border-left-color`, `color`), liens (`a`, `.internal-link`), gras et italique (`strong`, `em`), tableaux (`th`, `td` :
`border-color`, fond de l'en-tête). La chaîne de variables par défaut est la même que
dans Obsidian (`--color-base-00…100` → `--background-primary`…, `--accent-h/s/l` → `--interactive-accent`), donc un thème
qui ne change que quelques variables de base fonctionne.

Correspondance avec l'interface :

| Variable | Utilisée pour |
|---|---|
| `--background-primary` | fond de l'éditeur, du mode lecture et de la fenêtre des paramètres |
| `--background-secondary` | barres latérales, ruban, onglets, barre latérale des paramètres |
| `--background-modifier-border` | bordures et séparateurs |
| `--text-normal`, `--text-muted`, `--text-faint` | texte, descriptions, texte discret |
| `--interactive-accent`, `--color-accent` | couleur d'accent (remplacée par celle choisie dans les paramètres) |
| `--font-text-size` | taille du texte des notes (prioritaire sur le réglage) |
| `--file-line-width` | largeur de lecture |

Variables prises en charge : `--background-primary`, `--background-primary-alt`, `--background-secondary`,
`--background-modifier-border`, `--background-modifier-hover`, `--nav-item-background-hover/active`, `--text-normal`,
`--text-muted`, `--text-faint`, `--interactive-accent`, `--color-accent`, `--accent-h/s/l`, `--text-accent`, `--link-color`,
`--code-background`, `--code-normal`, `--tag-background`, `--tag-color`, `--checkbox-color`, `--blockquote-border-color`,
`--blockquote-color`, `--table-border-color`, `--table-header-background`, `--bold-color`, `--italic-color`,
`--h1-color`…`--h6-color`, `--h1-size`…`--h6-size`, `--font-text-size`, `--file-line-width`, `--line-height-normal`,
`--text-selection`, `--color-base-*`, `--mono-rgb-100`, et pour les polices `--font-text-theme`,
`--font-interface-theme`, `--font-monospace-theme` (si ni `appearance.json` ni les paramètres n'en choisissent).

`@media` : les blocs `print`, `speech`… sont ignorés ; `(prefers-color-scheme: dark|light)` ne s'applique qu'au mode
sombre ou clair ; `min-width` / `max-width` / `min-height` / `max-height` sont comparés à une fenêtre de bureau
(1280 × 800) ; les autres conditions sont tenues pour vraies. Une variable qui se référence elle-même (directement ou
non) est invalide, comme dans un navigateur : le `var()` qui la lit prend sa valeur de repli. Garde-fous : valeur résolue
de 16 Ko au plus, 2000 `var()` développés par valeur, 64 niveaux de parenthèses dans `calc()`, 32 blocs imbriqués.

En mode lecture, le frontmatter s'affiche en bloc « Propriétés » et les callouts Obsidian (`> [!info] Titre`, les 13
types et leurs alias, repliables avec `-` / `+`) avec leurs couleurs et icônes.

Limites (l'interface n'est pas une page web) : les autres règles qui ciblent des éléments précis
(`.nav-file-title { … }`, `:hover`…) ne sont pas appliquées ; le rapport de chargement les compte. Les polices
variables sont dessinées dans leur style par défaut uniquement ; `lab()` / `oklch()` ne sont pas lus.
