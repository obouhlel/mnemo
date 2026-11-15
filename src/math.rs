//! Formules LaTeX (`$…$`, `$$…$$`) rendues en texte Unicode lisible : lettres grecques, opérateurs, indices et
//! exposants, fractions, racines, ensembles (ℝ, ℕ…). Ce n'est pas KaTeX : les constructions sans équivalent
//! (matrices, alignements) restent lisibles en texte, sans commandes LaTeX.

/// Commandes sans argument → symbole.
const SYMBOLS: &[(&str, &str)] = &[
    ("alpha", "α"),
    ("beta", "β"),
    ("gamma", "γ"),
    ("delta", "δ"),
    ("epsilon", "ϵ"),
    ("varepsilon", "ε"),
    ("zeta", "ζ"),
    ("eta", "η"),
    ("theta", "θ"),
    ("vartheta", "ϑ"),
    ("iota", "ι"),
    ("kappa", "κ"),
    ("lambda", "λ"),
    ("mu", "μ"),
    ("nu", "ν"),
    ("xi", "ξ"),
    ("pi", "π"),
    ("varpi", "ϖ"),
    ("rho", "ρ"),
    ("varrho", "ϱ"),
    ("sigma", "σ"),
    ("varsigma", "ς"),
    ("tau", "τ"),
    ("upsilon", "υ"),
    ("phi", "ϕ"),
    ("varphi", "φ"),
    ("chi", "χ"),
    ("psi", "ψ"),
    ("omega", "ω"),
    ("Gamma", "Γ"),
    ("Delta", "Δ"),
    ("Theta", "Θ"),
    ("Lambda", "Λ"),
    ("Xi", "Ξ"),
    ("Pi", "Π"),
    ("Sigma", "Σ"),
    ("Upsilon", "Υ"),
    ("Phi", "Φ"),
    ("Psi", "Ψ"),
    ("Omega", "Ω"),
    ("sum", "∑"),
    ("prod", "∏"),
    ("coprod", "∐"),
    ("int", "∫"),
    ("iint", "∬"),
    ("iiint", "∭"),
    ("oint", "∮"),
    ("infty", "∞"),
    ("partial", "∂"),
    ("nabla", "∇"),
    ("pm", "±"),
    ("mp", "∓"),
    ("times", "×"),
    ("div", "÷"),
    ("cdot", "·"),
    ("cdots", "⋯"),
    ("ldots", "…"),
    ("dots", "…"),
    ("vdots", "⋮"),
    ("ddots", "⋱"),
    ("ast", "∗"),
    ("star", "⋆"),
    ("circ", "∘"),
    ("bullet", "•"),
    ("oplus", "⊕"),
    ("otimes", "⊗"),
    ("leq", "≤"),
    ("le", "≤"),
    ("geq", "≥"),
    ("ge", "≥"),
    ("neq", "≠"),
    ("ne", "≠"),
    ("approx", "≈"),
    ("equiv", "≡"),
    ("sim", "∼"),
    ("simeq", "≃"),
    ("cong", "≅"),
    ("propto", "∝"),
    ("ll", "≪"),
    ("gg", "≫"),
    ("in", "∈"),
    ("notin", "∉"),
    ("ni", "∋"),
    ("subset", "⊂"),
    ("supset", "⊃"),
    ("subseteq", "⊆"),
    ("supseteq", "⊇"),
    ("cup", "∪"),
    ("cap", "∩"),
    ("bigcup", "⋃"),
    ("bigcap", "⋂"),
    ("emptyset", "∅"),
    ("varnothing", "∅"),
    ("forall", "∀"),
    ("exists", "∃"),
    ("nexists", "∄"),
    ("neg", "¬"),
    ("lnot", "¬"),
    ("land", "∧"),
    ("wedge", "∧"),
    ("lor", "∨"),
    ("vee", "∨"),
    ("to", "→"),
    ("rightarrow", "→"),
    ("leftarrow", "←"),
    ("gets", "←"),
    ("leftrightarrow", "↔"),
    ("Rightarrow", "⇒"),
    ("implies", "⟹"),
    ("Leftarrow", "⇐"),
    ("Leftrightarrow", "⇔"),
    ("iff", "⟺"),
    ("mapsto", "↦"),
    ("uparrow", "↑"),
    ("downarrow", "↓"),
    ("longrightarrow", "⟶"),
    ("perp", "⊥"),
    ("parallel", "∥"),
    ("angle", "∠"),
    ("degree", "°"),
    ("prime", "′"),
    ("hbar", "ℏ"),
    ("ell", "ℓ"),
    ("Re", "ℜ"),
    ("Im", "ℑ"),
    ("aleph", "ℵ"),
    ("langle", "⟨"),
    ("rangle", "⟩"),
    ("lfloor", "⌊"),
    ("rfloor", "⌋"),
    ("lceil", "⌈"),
    ("rceil", "⌉"),
    ("vert", "|"),
    ("Vert", "‖"),
    ("mid", "∣"),
    ("setminus", "∖"),
    ("therefore", "∴"),
    ("because", "∵"),
    ("quad", "  "),
    ("qquad", "    "),
    (",", " "),
    (";", " "),
    (":", " "),
    ("!", ""),
    (" ", " "),
    ("{", "{"),
    ("}", "}"),
    ("%", "%"),
    ("$", "$"),
    ("&", "&"),
    ("#", "#"),
    ("_", "_"),
    ("\\", "\n"),
    ("lim", "lim"),
    ("sin", "sin"),
    ("cos", "cos"),
    ("tan", "tan"),
    ("log", "log"),
    ("ln", "ln"),
    ("exp", "exp"),
    ("max", "max"),
    ("min", "min"),
    ("sup", "sup"),
    ("inf", "inf"),
    ("det", "det"),
    ("gcd", "pgcd"),
    ("left", ""),
    ("right", ""),
    ("big", ""),
    ("Big", ""),
    ("bigg", ""),
    ("Bigg", ""),
    ("displaystyle", ""),
    ("limits", ""),
    ("nolimits", ""),
];

const BLACKBOARD: &[(char, &str)] =
    &[('R', "ℝ"), ('N', "ℕ"), ('Z', "ℤ"), ('Q', "ℚ"), ('C', "ℂ"), ('P', "ℙ"), ('E', "𝔼"), ('H', "ℍ"), ('1', "𝟙")];

fn sup(c: char) -> Option<char> {
    Some(match c {
        '0' => '⁰',
        '1' => '¹',
        '2' => '²',
        '3' => '³',
        '4' => '⁴',
        '5' => '⁵',
        '6' => '⁶',
        '7' => '⁷',
        '8' => '⁸',
        '9' => '⁹',
        '+' => '⁺',
        '-' => '⁻',
        '−' => '⁻',
        '=' => '⁼',
        '(' => '⁽',
        ')' => '⁾',
        'n' => 'ⁿ',
        'i' => 'ⁱ',
        'a' => 'ᵃ',
        'b' => 'ᵇ',
        'c' => 'ᶜ',
        'd' => 'ᵈ',
        'e' => 'ᵉ',
        'f' => 'ᶠ',
        'g' => 'ᵍ',
        'h' => 'ʰ',
        'j' => 'ʲ',
        'k' => 'ᵏ',
        'l' => 'ˡ',
        'm' => 'ᵐ',
        'o' => 'ᵒ',
        'p' => 'ᵖ',
        'r' => 'ʳ',
        's' => 'ˢ',
        't' => 'ᵗ',
        'u' => 'ᵘ',
        'v' => 'ᵛ',
        'w' => 'ʷ',
        'x' => 'ˣ',
        'y' => 'ʸ',
        'z' => 'ᶻ',
        'T' => 'ᵀ',
        '′' => '′',
        '*' => '*',
        ' ' => ' ',
        _ => return None,
    })
}

fn sub(c: char) -> Option<char> {
    Some(match c {
        '0' => '₀',
        '1' => '₁',
        '2' => '₂',
        '3' => '₃',
        '4' => '₄',
        '5' => '₅',
        '6' => '₆',
        '7' => '₇',
        '8' => '₈',
        '9' => '₉',
        '+' => '₊',
        '-' => '₋',
        '−' => '₋',
        '=' => '₌',
        '(' => '₍',
        ')' => '₎',
        'a' => 'ₐ',
        'e' => 'ₑ',
        'h' => 'ₕ',
        'i' => 'ᵢ',
        'j' => 'ⱼ',
        'k' => 'ₖ',
        'l' => 'ₗ',
        'm' => 'ₘ',
        'n' => 'ₙ',
        'o' => 'ₒ',
        'p' => 'ₚ',
        'r' => 'ᵣ',
        's' => 'ₛ',
        't' => 'ₜ',
        'u' => 'ᵤ',
        'v' => 'ᵥ',
        'x' => 'ₓ',
        ' ' => ' ',
        _ => return None,
    })
}

struct Parser<'a> {
    s: &'a [char],
    i: usize,
}

impl Parser<'_> {
    /// Argument d'une commande : `{…}` (converti) ou un seul caractère / une seule commande.
    fn arg(&mut self) -> String {
        while self.s.get(self.i) == Some(&' ') {
            self.i += 1;
        }
        match self.s.get(self.i) {
            Some('{') => {
                self.i += 1;
                let start = self.i;
                let mut depth = 1;
                while let Some(&c) = self.s.get(self.i) {
                    match c {
                        '\\' => self.i += 1,
                        '{' => depth += 1,
                        '}' => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                    self.i += 1;
                }
                let inner: String = self.s[start..self.i.min(self.s.len())].iter().collect();
                self.i += 1;
                inner_convert(&inner)
            }
            Some('\\') => {
                let start = self.i;
                self.i += 1;
                self.command_name();
                inner_convert(&self.s[start..self.i].iter().collect::<String>())
            }
            Some(&c) => {
                self.i += 1;
                c.to_string()
            }
            None => String::new(),
        }
    }

    /// Argument brut (`\text{…}`, `\mathrm{…}`) : sans conversion.
    fn raw_arg(&mut self) -> String {
        if self.s.get(self.i) != Some(&'{') {
            return self.arg();
        }
        let start = self.i + 1;
        let mut depth = 0;
        while let Some(&c) = self.s.get(self.i) {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
            self.i += 1;
        }
        let out = self.s[start..self.i.min(self.s.len())].iter().collect();
        self.i += 1;
        out
    }

    fn command_name(&mut self) -> String {
        let start = self.i;
        if self.s.get(self.i).is_some_and(|c| c.is_ascii_alphabetic()) {
            while self.s.get(self.i).is_some_and(|c| c.is_ascii_alphabetic()) {
                self.i += 1;
            }
        } else if self.i < self.s.len() {
            self.i += 1;
        }
        self.s[start..self.i].iter().collect()
    }
}

/// Entoure de parenthèses un groupe de plus d'un symbole (`a+b` → `(a+b)`), pour les fractions.
fn group(s: &str) -> String {
    let s = s.trim();
    if s.chars().count() <= 1 || s.chars().all(|c| c.is_alphanumeric()) { s.to_string() } else { format!("({s})") }
}

fn script(s: &str, map: fn(char) -> Option<char>, mark: char) -> String {
    match s.chars().map(map).collect::<Option<String>>() {
        Some(t) => t,
        None if s.chars().count() == 1 => format!("{mark}{s}"),
        None => format!("{mark}({s})"),
    }
}

/// Formule LaTeX → texte Unicode.
pub fn convert(src: &str) -> String {
    space_relations(&inner_convert(src))
}

fn inner_convert(src: &str) -> String {
    let chars: Vec<char> = src.chars().collect();
    let mut p = Parser { s: &chars, i: 0 };
    let mut out = String::new();
    while p.i < chars.len() {
        let c = chars[p.i];
        match c {
            '\\' => {
                p.i += 1;
                let name = p.command_name();
                match name.as_str() {
                    "frac" | "dfrac" | "tfrac" => {
                        let (a, b) = (p.arg(), p.arg());
                        out.push_str(&format!("{}⁄{}", group(&a), group(&b)));
                    }
                    "sqrt" => {
                        let n = if chars.get(p.i) == Some(&'[') {
                            let end = chars[p.i..].iter().position(|c| *c == ']').map_or(chars.len(), |e| p.i + e);
                            let n: String = chars[p.i + 1..end].iter().collect();
                            p.i = end + 1;
                            Some(n)
                        } else {
                            None
                        };
                        let a = p.arg();
                        let root = match n.as_deref() {
                            Some("3") => "∛",
                            Some("4") => "∜",
                            Some(n) => {
                                out.push_str(&script(n, sup, '^'));
                                "√"
                            }
                            None => "√",
                        };
                        out.push_str(&format!("{root}{}", group(&a)));
                    }
                    "text" | "textrm" | "mathrm" | "operatorname" | "textit" | "textbf" | "mbox" | "mathit" => {
                        out.push_str(&p.raw_arg())
                    }
                    "mathbb" | "Bbb" => {
                        let a = p.raw_arg();
                        for ch in a.chars() {
                            match BLACKBOARD.iter().find(|(k, _)| *k == ch) {
                                Some((_, v)) => out.push_str(v),
                                None => out.push(ch),
                            }
                        }
                    }
                    "mathbf" | "boldsymbol" | "mathcal" | "mathscr" | "mathfrak" | "vec" | "overline" | "bar"
                    | "hat" | "tilde" | "dot" | "ddot" | "underline" | "widehat" | "widetilde" | "overrightarrow" => {
                        let a = p.arg();
                        let accent = match name.as_str() {
                            "vec" | "overrightarrow" => "\u{20d7}",
                            "overline" | "bar" => "\u{305}",
                            "hat" | "widehat" => "\u{302}",
                            "tilde" | "widetilde" => "\u{303}",
                            "dot" => "\u{307}",
                            "ddot" => "\u{308}",
                            "underline" => "\u{332}",
                            _ => "",
                        };
                        out.push_str(&a);
                        out.push_str(accent);
                    }
                    "begin" | "end" => {
                        p.raw_arg();
                    }
                    "binom" => {
                        let (a, b) = (p.arg(), p.arg());
                        out.push_str(&format!("C({a}, {b})"));
                    }
                    _ => match SYMBOLS.iter().find(|(k, _)| *k == name) {
                        Some((_, v)) => out.push_str(v),
                        None => out.push_str(&name),
                    },
                }
            }
            '^' => {
                p.i += 1;
                let a = p.arg();
                out.push_str(&script(&a, sup, '^'));
            }
            '_' => {
                p.i += 1;
                let a = p.arg();
                out.push_str(&script(&a, sub, '_'));
            }
            '{' | '}' => p.i += 1,
            '&' => {
                out.push(' ');
                p.i += 1;
            }
            '~' => {
                out.push(' ');
                p.i += 1;
            }
            '-' => {
                out.push('−');
                p.i += 1;
            }
            '*' => {
                out.push('∗');
                p.i += 1;
            }
            _ => {
                out.push(c);
                p.i += 1;
            }
        }
    }
    out
}

/// Espaces autour des relations (=, ≤, →…), comme un rendu mathématique.
fn space_relations(out: &str) -> String {
    let mut spaced = String::with_capacity(out.len());
    for ch in out.chars() {
        if matches!(ch, '=' | '≤' | '≥' | '≠' | '≈' | '→' | '⇒' | '⟹' | '⇔' | '∈' | '≡') {
            if !spaced.ends_with(' ') && !spaced.is_empty() {
                spaced.push(' ');
            }
            spaced.push(ch);
            spaced.push(' ');
        } else if ch == ' ' && spaced.ends_with(' ') {
        } else {
            spaced.push(ch);
        }
    }
    spaced.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::convert;

    #[test]
    fn formulas() {
        assert_eq!(convert("x^2 + y_1"), "x² + y₁");
        assert_eq!(convert(r"\frac{a+b}{2}"), "(a+b)⁄2");
        assert_eq!(convert(r"\sum_{i=0}^{n} i"), "∑ᵢ₌₀ⁿ i");
        assert_eq!(convert(r"\forall x \in \mathbb{R}, x^2 \geq 0"), "∀ x ∈ ℝ, x² ≥ 0");
        assert_eq!(convert(r"\sqrt{x}"), "√x");
        assert_eq!(convert(r"\sqrt[3]{x+1}"), "∛(x+1)");
        assert_eq!(convert(r"e^{i\pi} = -1"), "e^(iπ) = −1");
        assert_eq!(convert(r"\text{si } a \neq b"), "si a ≠ b");
        assert_eq!(convert(r"\left( \alpha \right)"), "( α )");
        assert_eq!(convert(r"\inconnu"), "inconnu");
    }
}
