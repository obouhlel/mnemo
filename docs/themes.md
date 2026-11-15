# Thèmes et snippets CSS (format Obsidian)

Mnemo lit les thèmes et snippets CSS au format Obsidian et applique leurs **variables CSS** à l'interface native.
Paramètres › Apparence :

- **Dossier `.obsidian` d'un coffre existant** (facultatif) : ses `themes/<Nom>/theme.css` (ou `themes/<Nom>.css`),
  `snippets/*.css` et son `appearance.json` (thème choisi, snippets activés, `baseFontSize`, `accentColor`,
  clair/sombre, polices `textFontFamily` / `interfaceFontFamily` / `monospaceFontFamily`, `showRibbon`) sont repris
  tels quels, ainsi que `app.json` (`showInlineTitle`, `readableLineLength`).
- Les polices sont cherchées parmi celles installées sur le système (nom exact de la famille). La police du texte
  s'applique à l'éditeur, à l'aperçu et au titre de la note ; l'interface garde la sienne, comme dans Obsidian.
  Les emoji sont dessinés en couleur si une police d'emoji couleur est installée (Noto Color Emoji, Segoe UI Emoji…).
- Dossiers propres à Mnemo, même structure : `<config>/themes/` et `<config>/snippets/`
  (un snippet d'exemple `exemple-accent-vert.css` y est installé).
- Les fichiers sont **rechargés à chaud** : modifiez un `.css`, l'interface change dans la seconde.

Le moteur comprend `var()` (avec valeur de repli), `calc()`, `hsl()/hsla()`, `rgb()/rgba()`, `color-mix()` (mélangé en
sRGB), hex, `@media`/`@supports`, et les déclarations sur `body`, `:root`, `.theme-dark`, `.theme-light`.
Les règles qui visent les éléments du texte sont traduites en variables : titres (`.markdown-rendered h1`,
`.cm-header-1`… : `color`, `font-size`), code en ligne (`code`, `.cm-inline-code`), citations (`blockquote` :
`border-left-color`, `color`), liens (`a`, `.internal-link`), gras et italique (`strong`, `em`), tableaux (`th`, `td` :
`border-color`, fond de l'en-tête). La chaîne de variables par défaut est la même que
dans Obsidian (`--color-base-00…100` → `--background-primary`…, `--accent-h/s/l` → `--interactive-accent`), donc un thème
qui ne change que quelques variables de base fonctionne.

Variables prises en charge : `--background-primary`, `--background-primary-alt`, `--background-secondary`,
`--background-modifier-border`, `--background-modifier-hover`, `--nav-item-background-hover/active`, `--text-normal`,
`--text-muted`, `--text-faint`, `--interactive-accent`, `--color-accent`, `--accent-h/s/l`, `--text-accent`, `--link-color`,
`--code-background`, `--code-normal`, `--tag-background`, `--tag-color`, `--checkbox-color`, `--blockquote-border-color`,
`--blockquote-color`, `--table-border-color`, `--table-header-background`, `--bold-color`, `--italic-color`,
`--h1-color`…`--h6-color`, `--h1-size`…`--h6-size`, `--font-text-size`, `--file-line-width`, `--line-height-normal`,
`--text-selection`, `--color-base-*`, `--mono-rgb-100`, et pour les polices `--font-text-theme`,
`--font-interface-theme`, `--font-monospace-theme` (si `appearance.json` n'en choisit pas).

`@media` : les blocs `print`, `speech`… sont ignorés ; `(prefers-color-scheme: dark|light)` ne s'applique qu'au mode
sombre ou clair ; `min-width` / `max-width` / `min-height` / `max-height` sont comparés à une fenêtre de bureau
(1280 × 800) ; les autres conditions sont tenues pour vraies. Une variable qui se référence elle-même (directement ou
non) est invalide, comme dans un navigateur : le `var()` qui la lit prend sa valeur de repli. Garde-fous : valeur résolue
de 16 Ko au plus, 2000 `var()` développés par valeur, 64 niveaux de parenthèses dans `calc()`, 32 blocs imbriqués.

En mode lecture, le frontmatter s'affiche en bloc « Propriétés » et les callouts Obsidian (`> [!info] Titre`, les 13
types et leurs alias, repliables avec `-` / `+`) avec leurs couleurs et icônes.

Limites (l'interface n'est pas une page web) : les autres règles qui ciblent des éléments précis
(`.nav-file-title { … }`, `:hover`…) ne sont pas appliquées ; le rapport des paramètres les compte. Les polices
variables sont dessinées dans leur style par défaut uniquement ; `lab()` / `oklch()` ne sont pas lus.
