# Thèmes et snippets CSS (format Obsidian)

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

`@media` : les blocs `print`, `speech`… sont ignorés ; `(prefers-color-scheme: dark|light)` ne s'applique qu'au mode
sombre ou clair ; `min-width` / `max-width` / `min-height` / `max-height` sont comparés à une fenêtre de bureau
(1280 × 800) ; les autres conditions sont tenues pour vraies. Une variable qui se référence elle-même (directement ou
non) est invalide, comme dans un navigateur : le `var()` qui la lit prend sa valeur de repli. Garde-fous : valeur résolue
de 16 Ko au plus, 2000 `var()` développés par valeur, 64 niveaux de parenthèses dans `calc()`, 32 blocs imbriqués.

Limites (l'interface n'est pas une page web) : les règles qui ciblent des éléments précis (`.nav-file-title { … }`),
`color-mix()` et les polices personnalisées ne sont pas appliquées ; le rapport des paramètres les signale.
