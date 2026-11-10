//! A tiny math expression language for animating properties, compiled once and evaluated per frame.
//!
//! `13 * exp(-t * 9) * sin(t * 38 + 1.7)`, `clamp((t - i * 0.05) / 0.3, 0, 1)`, `t < 1 ? outExpo(t) : 1`

use anyhow::{Result, anyhow, bail};

/// Variables an expression can read. Unknown names are rejected at parse time.
#[derive(Clone, Copy, Default)]
pub struct Vars {
    /// Layer-local time in seconds.
    pub t: f64,
    /// Composition time in seconds.
    pub time: f64,
    /// Composition frame number.
    pub frame: f64,
    /// The layer's duration in seconds.
    pub dur: f64,
    /// Glyph index and glyph count (text `glyphs` properties only).
    pub i: f64,
    pub n: f64,
}

#[derive(Clone, Copy, Debug)]
enum Var {
    T,
    Time,
    Frame,
    Dur,
    I,
    N,
}

#[derive(Clone, Copy, Debug)]
enum Op {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
    And,
    Or,
}

#[derive(Clone, Debug)]
enum Node {
    Num(f64),
    Var(Var),
    Neg(Box<Node>),
    Not(Box<Node>),
    Bin(Op, Box<Node>, Box<Node>),
    Cond(Box<Node>, Box<Node>, Box<Node>),
    Call(Func, Vec<Node>),
}

#[derive(Clone, Debug)]
pub struct Expr {
    root: Node,
}

macro_rules! funcs {
    ($( $name:literal => $variant:ident / $arity:expr ),* $(,)?) => {
        #[derive(Clone, Copy, Debug)]
        enum Func { $($variant),* }
        fn lookup(name: &str) -> Option<(Func, usize)> {
            match name { $($name => Some((Func::$variant, $arity)),)* _ => None }
        }
        pub const FUNCTION_NAMES: &[&str] = &[$($name),*];
    };
}

funcs! {
    "sin" => Sin/1, "cos" => Cos/1, "tan" => Tan/1, "asin" => Asin/1, "acos" => Acos/1, "atan" => Atan/1,
    "atan2" => Atan2/2, "sqrt" => Sqrt/1, "exp" => Exp/1, "log" => Log/1, "pow" => Pow/2, "abs" => Abs/1,
    "min" => Min/2, "max" => Max/2, "clamp" => Clamp/3, "floor" => Floor/1, "ceil" => Ceil/1,
    "round" => Round/1, "fract" => Fract/1, "sign" => Sign/1, "mix" => Mix/3, "lerp" => Lerp/3,
    "step" => Step/2, "smoothstep" => Smoothstep/3, "remap" => Remap/5,
    "rand" => Rand/1, "noise" => Noise/1, "noise2" => Noise2/2,
    "linear" => Linear/1,
    "inSine" => InSine/1, "outSine" => OutSine/1, "inOutSine" => InOutSine/1,
    "inQuad" => InQuad/1, "outQuad" => OutQuad/1, "inOutQuad" => InOutQuad/1,
    "inCubic" => InCubic/1, "outCubic" => OutCubic/1, "inOutCubic" => InOutCubic/1,
    "inQuart" => InQuart/1, "outQuart" => OutQuart/1, "inOutQuart" => InOutQuart/1,
    "inExpo" => InExpo/1, "outExpo" => OutExpo/1, "inOutExpo" => InOutExpo/1,
    "inBack" => InBack/1, "outBack" => OutBack/1, "inOutBack" => InOutBack/1,
    "outElastic" => OutElastic/1, "outBounce" => OutBounce/1,
}

/// Easing by name, for keyframes. Input is clamped to 0..1.
pub fn ease(name: &str, x: f64) -> Option<f64> {
    let (f, arity) = lookup(name)?;
    if arity != 1 || !is_ease(f) {
        return None;
    }
    Some(call(f, &[x]))
}

pub fn is_ease_name(name: &str) -> bool {
    matches!(lookup(name), Some((f, 1)) if is_ease(f))
}

fn is_ease(f: Func) -> bool {
    use Func::*;
    matches!(
        f,
        Linear
            | InSine
            | OutSine
            | InOutSine
            | InQuad
            | OutQuad
            | InOutQuad
            | InCubic
            | OutCubic
            | InOutCubic
            | InQuart
            | OutQuart
            | InOutQuart
            | InExpo
            | OutExpo
            | InOutExpo
            | InBack
            | OutBack
            | InOutBack
            | OutElastic
            | OutBounce
    )
}

fn hash(x: f64) -> f64 {
    // A stable 0..1 hash of a number, the same on every platform.
    let mut h = (x * 1000.0).round() as i64 as u64 ^ 0x9E37_79B9_7F4A_7C15;
    h ^= h >> 33;
    h = h.wrapping_mul(0xff51_afd7_ed55_8ccd);
    h ^= h >> 33;
    h = h.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
    h ^= h >> 33;
    (h >> 11) as f64 / (1u64 << 53) as f64
}

fn ihash(a: i64, b: i64) -> f64 {
    hash(a as f64 * 7919.0 + b as f64 * 104_729.0)
}

/// Smooth value noise in -1..1.
fn noise1(x: f64) -> f64 {
    let i = x.floor();
    let f = x - i;
    let u = f * f * (3.0 - 2.0 * f);
    let a = ihash(i as i64, 0);
    let b = ihash(i as i64 + 1, 0);
    (a + (b - a) * u) * 2.0 - 1.0
}

fn noise2(x: f64, y: f64) -> f64 {
    let (ix, iy) = (x.floor(), y.floor());
    let (fx, fy) = (x - ix, y - iy);
    let (ux, uy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    let (ix, iy) = (ix as i64, iy as i64);
    let a = ihash(ix, iy);
    let b = ihash(ix + 1, iy);
    let c = ihash(ix, iy + 1);
    let d = ihash(ix + 1, iy + 1);
    let top = a + (b - a) * ux;
    let bot = c + (d - c) * ux;
    (top + (bot - top) * uy) * 2.0 - 1.0
}

fn call(f: Func, a: &[f64]) -> f64 {
    use Func::*;
    use std::f64::consts::PI;
    let c = |x: f64| x.clamp(0.0, 1.0);
    match f {
        Sin => a[0].sin(),
        Cos => a[0].cos(),
        Tan => a[0].tan(),
        Asin => a[0].asin(),
        Acos => a[0].acos(),
        Atan => a[0].atan(),
        Atan2 => a[0].atan2(a[1]),
        Sqrt => a[0].sqrt(),
        Exp => a[0].exp(),
        Log => a[0].ln(),
        Pow => a[0].powf(a[1]),
        Abs => a[0].abs(),
        Min => a[0].min(a[1]),
        Max => a[0].max(a[1]),
        Clamp => a[0].clamp(a[1].min(a[2]), a[2].max(a[1])),
        Floor => a[0].floor(),
        Ceil => a[0].ceil(),
        Round => a[0].round(),
        Fract => a[0] - a[0].floor(),
        Sign => {
            if a[0] > 0.0 {
                1.0
            } else if a[0] < 0.0 {
                -1.0
            } else {
                0.0
            }
        }
        Mix | Lerp => a[0] + (a[1] - a[0]) * a[2],
        Step => {
            if a[1] >= a[0] {
                1.0
            } else {
                0.0
            }
        }
        Smoothstep => {
            let x = c((a[2] - a[0]) / (a[1] - a[0]));
            x * x * (3.0 - 2.0 * x)
        }
        Remap => a[3] + (a[4] - a[3]) * c((a[0] - a[1]) / (a[2] - a[1])),
        Rand => hash(a[0]),
        Noise => noise1(a[0]),
        Noise2 => noise2(a[0], a[1]),
        Linear => c(a[0]),
        InSine => 1.0 - (c(a[0]) * PI / 2.0).cos(),
        OutSine => (c(a[0]) * PI / 2.0).sin(),
        InOutSine => -((PI * c(a[0])).cos() - 1.0) / 2.0,
        InQuad => c(a[0]).powi(2),
        OutQuad => 1.0 - (1.0 - c(a[0])).powi(2),
        InOutQuad => {
            let x = c(a[0]);
            if x < 0.5 { 2.0 * x * x } else { 1.0 - (-2.0 * x + 2.0).powi(2) / 2.0 }
        }
        InCubic => c(a[0]).powi(3),
        OutCubic => 1.0 - (1.0 - c(a[0])).powi(3),
        InOutCubic => {
            let x = c(a[0]);
            if x < 0.5 { 4.0 * x * x * x } else { 1.0 - (-2.0 * x + 2.0).powi(3) / 2.0 }
        }
        InQuart => c(a[0]).powi(4),
        OutQuart => 1.0 - (1.0 - c(a[0])).powi(4),
        InOutQuart => {
            let x = c(a[0]);
            if x < 0.5 { 8.0 * x.powi(4) } else { 1.0 - (-2.0 * x + 2.0).powi(4) / 2.0 }
        }
        InExpo => {
            let x = c(a[0]);
            if x <= 0.0 { 0.0 } else { 2f64.powf(10.0 * x - 10.0) }
        }
        OutExpo => {
            let x = c(a[0]);
            if x >= 1.0 { 1.0 } else { 1.0 - 2f64.powf(-10.0 * x) }
        }
        InOutExpo => {
            let x = c(a[0]);
            if x <= 0.0 {
                0.0
            } else if x >= 1.0 {
                1.0
            } else if x < 0.5 {
                2f64.powf(20.0 * x - 10.0) / 2.0
            } else {
                (2.0 - 2f64.powf(-20.0 * x + 10.0)) / 2.0
            }
        }
        InBack => {
            let x = c(a[0]);
            2.70158 * x.powi(3) - 1.70158 * x * x
        }
        OutBack => {
            let x = c(a[0]) - 1.0;
            1.0 + 2.70158 * x.powi(3) + 1.70158 * x * x
        }
        InOutBack => {
            let x = c(a[0]);
            let k = 1.70158 * 1.525;
            if x < 0.5 {
                (2.0 * x).powi(2) * ((k + 1.0) * 2.0 * x - k) / 2.0
            } else {
                ((2.0 * x - 2.0).powi(2) * ((k + 1.0) * (x * 2.0 - 2.0) + k) + 2.0) / 2.0
            }
        }
        OutElastic => {
            let x = c(a[0]);
            if x <= 0.0 || x >= 1.0 {
                x
            } else {
                2f64.powf(-10.0 * x) * ((x * 10.0 - 0.75) * (2.0 * PI / 3.0)).sin() + 1.0
            }
        }
        OutBounce => {
            let mut x = c(a[0]);
            let (n, d) = (7.5625, 2.75);
            if x < 1.0 / d {
                n * x * x
            } else if x < 2.0 / d {
                x -= 1.5 / d;
                n * x * x + 0.75
            } else if x < 2.5 / d {
                x -= 2.25 / d;
                n * x * x + 0.9375
            } else {
                x -= 2.625 / d;
                n * x * x + 0.984375
            }
        }
    }
}

impl Expr {
    pub fn parse(src: &str) -> Result<Expr> {
        let toks = lex(src)?;
        let mut p = Parser { toks, pos: 0 };
        let root = p.ternary()?;
        if p.pos != p.toks.len() {
            bail!("unexpected `{}` in expression `{src}`", p.toks[p.pos].text());
        }
        Ok(Expr { root })
    }

    pub fn eval(&self, v: &Vars) -> f64 {
        eval(&self.root, v)
    }

    /// The constant value, if the expression reads no variables.
    pub fn constant(&self) -> Option<f64> {
        (!reads(&self.root, &|_| true)).then(|| eval(&self.root, &Vars::default()))
    }

    /// Whether the expression reads `i` or `n`, which only text `glyphs` properties define.
    pub fn reads_glyph_vars(&self) -> bool {
        reads(&self.root, &|v| matches!(v, Var::I | Var::N))
    }
}

fn reads(n: &Node, which: &dyn Fn(Var) -> bool) -> bool {
    match n {
        Node::Num(_) => false,
        Node::Var(v) => which(*v),
        Node::Neg(a) | Node::Not(a) => reads(a, which),
        Node::Bin(_, a, b) => reads(a, which) || reads(b, which),
        Node::Cond(a, b, c) => reads(a, which) || reads(b, which) || reads(c, which),
        Node::Call(_, args) => args.iter().any(|a| reads(a, which)),
    }
}

fn eval(n: &Node, v: &Vars) -> f64 {
    let b = |x: bool| if x { 1.0 } else { 0.0 };
    match n {
        Node::Num(x) => *x,
        Node::Var(var) => match var {
            Var::T => v.t,
            Var::Time => v.time,
            Var::Frame => v.frame,
            Var::Dur => v.dur,
            Var::I => v.i,
            Var::N => v.n,
        },
        Node::Neg(a) => -eval(a, v),
        Node::Not(a) => b(eval(a, v) == 0.0),
        Node::Bin(op, l, r) => {
            let x = eval(l, v);
            match op {
                Op::And => return b(x != 0.0 && eval(r, v) != 0.0),
                Op::Or => return b(x != 0.0 || eval(r, v) != 0.0),
                _ => {}
            }
            let y = eval(r, v);
            match op {
                Op::Add => x + y,
                Op::Sub => x - y,
                Op::Mul => x * y,
                Op::Div => x / y,
                Op::Mod => x.rem_euclid(y),
                Op::Pow => x.powf(y),
                Op::Lt => b(x < y),
                Op::Le => b(x <= y),
                Op::Gt => b(x > y),
                Op::Ge => b(x >= y),
                Op::Eq => b(x == y),
                Op::Ne => b(x != y),
                Op::And | Op::Or => unreachable!(),
            }
        }
        Node::Cond(c, a, e) => {
            if eval(c, v) != 0.0 {
                eval(a, v)
            } else {
                eval(e, v)
            }
        }
        Node::Call(f, args) => {
            let mut a = [0.0; 5];
            for (k, arg) in args.iter().enumerate() {
                a[k] = eval(arg, v);
            }
            call(*f, &a[..args.len()])
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Num(f64),
    Ident(String),
    Sym(&'static str),
}

impl Tok {
    fn text(&self) -> String {
        match self {
            Tok::Num(x) => x.to_string(),
            Tok::Ident(s) => s.clone(),
            Tok::Sym(s) => s.to_string(),
        }
    }
}

fn lex(src: &str) -> Result<Vec<Tok>> {
    const SYMS: &[&str] = &[
        "<=", ">=", "==", "!=", "&&", "||", "**", "+", "-", "*", "/", "%", "^", "(", ")", ",", "<", ">", "?", ":", "!",
    ];
    let b = src.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    'outer: while i < b.len() {
        let c = b[i] as char;
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if c.is_ascii_digit() || (c == '.' && i + 1 < b.len() && (b[i + 1] as char).is_ascii_digit()) {
            let s = i;
            while i < b.len() && ((b[i] as char).is_ascii_digit() || b[i] == b'.') {
                i += 1;
            }
            if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
                let save = i;
                i += 1;
                if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
                    i += 1;
                }
                if i < b.len() && (b[i] as char).is_ascii_digit() {
                    while i < b.len() && (b[i] as char).is_ascii_digit() {
                        i += 1;
                    }
                } else {
                    i = save;
                }
            }
            out.push(Tok::Num(src[s..i].parse().map_err(|_| anyhow!("bad number `{}` in `{src}`", &src[s..i]))?));
            continue;
        }
        if c.is_ascii_alphabetic() || c == '_' {
            let s = i;
            while i < b.len() && ((b[i] as char).is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
            out.push(Tok::Ident(src[s..i].to_string()));
            continue;
        }
        for s in SYMS {
            if src[i..].starts_with(s) {
                out.push(Tok::Sym(if *s == "**" { "^" } else { s }));
                i += s.len();
                continue 'outer;
            }
        }
        bail!("unexpected character `{c}` in expression `{src}`");
    }
    Ok(out)
}

struct Parser {
    toks: Vec<Tok>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos)
    }
    fn eat(&mut self, s: &str) -> bool {
        if matches!(self.peek(), Some(Tok::Sym(x)) if *x == s) {
            self.pos += 1;
            true
        } else {
            false
        }
    }
    fn expect(&mut self, s: &str) -> Result<()> {
        if self.eat(s) {
            Ok(())
        } else {
            bail!(
                "expected `{s}`, found {}",
                self.peek().map(|t| format!("`{}`", t.text())).unwrap_or("the end".into())
            )
        }
    }
    fn ternary(&mut self) -> Result<Node> {
        let c = self.binary(0)?;
        if self.eat("?") {
            let a = self.ternary()?;
            self.expect(":")?;
            let e = self.ternary()?;
            return Ok(Node::Cond(Box::new(c), Box::new(a), Box::new(e)));
        }
        Ok(c)
    }
    fn binary(&mut self, min: u8) -> Result<Node> {
        let mut l = self.unary()?;
        while let Some(Tok::Sym(s)) = self.peek() {
            let (op, prec, right) = match *s {
                "||" => (Op::Or, 1, false),
                "&&" => (Op::And, 2, false),
                "==" => (Op::Eq, 3, false),
                "!=" => (Op::Ne, 3, false),
                "<" => (Op::Lt, 4, false),
                "<=" => (Op::Le, 4, false),
                ">" => (Op::Gt, 4, false),
                ">=" => (Op::Ge, 4, false),
                "+" => (Op::Add, 5, false),
                "-" => (Op::Sub, 5, false),
                "*" => (Op::Mul, 6, false),
                "/" => (Op::Div, 6, false),
                "%" => (Op::Mod, 6, false),
                "^" => (Op::Pow, 8, true),
                _ => break,
            };
            if prec < min {
                break;
            }
            self.pos += 1;
            let r = self.binary(if right { prec } else { prec + 1 })?;
            l = Node::Bin(op, Box::new(l), Box::new(r));
        }
        Ok(l)
    }
    fn unary(&mut self) -> Result<Node> {
        if self.eat("-") {
            // binds looser than ^ so -2^2 == -4
            return Ok(Node::Neg(Box::new(self.binary(7)?)));
        }
        if self.eat("+") {
            return self.binary(7);
        }
        if self.eat("!") {
            return Ok(Node::Not(Box::new(self.binary(7)?)));
        }
        self.atom()
    }
    fn atom(&mut self) -> Result<Node> {
        let tok = self.toks.get(self.pos).cloned().ok_or_else(|| anyhow!("expression ends too early"))?;
        self.pos += 1;
        match tok {
            Tok::Num(x) => Ok(Node::Num(x)),
            Tok::Sym("(") => {
                let e = self.ternary()?;
                self.expect(")")?;
                Ok(e)
            }
            Tok::Ident(name) => {
                if self.eat("(") {
                    let (f, arity) = lookup(&name).ok_or_else(|| {
                        anyhow!("unknown function `{name}`; available: {}", FUNCTION_NAMES.join(", "))
                    })?;
                    let mut args = Vec::new();
                    if !self.eat(")") {
                        loop {
                            args.push(self.ternary()?);
                            if self.eat(")") {
                                break;
                            }
                            self.expect(",")?;
                        }
                    }
                    if args.len() != arity {
                        bail!("`{name}` takes {arity} argument(s), got {}", args.len());
                    }
                    return Ok(Node::Call(f, args));
                }
                let var = match name.as_str() {
                    "t" => Var::T,
                    "time" => Var::Time,
                    "frame" => Var::Frame,
                    "dur" => Var::Dur,
                    "i" => Var::I,
                    "n" => Var::N,
                    "pi" | "PI" => return Ok(Node::Num(std::f64::consts::PI)),
                    "e" | "E" => return Ok(Node::Num(std::f64::consts::E)),
                    _ => bail!("unknown variable `{name}`; available: t, time, frame, dur, i, n, pi, e"),
                };
                Ok(Node::Var(var))
            }
            Tok::Sym(s) => bail!("unexpected `{s}`"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(s: &str, t: f64) -> f64 {
        Expr::parse(s).unwrap().eval(&Vars { t, ..Default::default() })
    }

    #[test]
    fn arithmetic_and_precedence() {
        assert_eq!(ev("1 + 2 * 3", 0.0), 7.0);
        assert_eq!(ev("-2^2", 0.0), -4.0);
        assert_eq!(ev("2^3^2", 0.0), 512.0);
        assert_eq!(ev("(1 + 2) * 3", 0.0), 9.0);
        assert_eq!(ev("t > 1 ? 10 : 20", 2.0), 10.0);
        assert_eq!(ev("7 % 3", 0.0), 1.0);
        assert_eq!(ev("1.5e2", 0.0), 150.0);
    }

    #[test]
    fn functions() {
        assert!((ev("13 * exp(-t * 9) * sin(t * 38 + 1.7)", 0.0) - 13.0 * 1.7f64.sin()).abs() < 1e-9);
        assert_eq!(ev("clamp(t, 0, 1)", 3.0), 1.0);
        assert_eq!(ev("outExpo(t)", 1.0), 1.0);
        assert_eq!(ev("remap(t, 0, 2, 10, 20)", 1.0), 15.0);
        assert!(ev("noise(t)", 0.3).abs() <= 1.0);
    }

    #[test]
    fn errors_name_the_problem() {
        assert!(Expr::parse("foo(t)").unwrap_err().to_string().contains("unknown function"));
        assert!(Expr::parse("x + 1").unwrap_err().to_string().contains("unknown variable"));
        assert!(Expr::parse("sin(1, 2)").unwrap_err().to_string().contains("takes 1"));
        assert!(Expr::parse("(1 + 2").is_err());
    }

    #[test]
    fn constants_fold() {
        assert_eq!(Expr::parse("2 * pi").unwrap().constant(), Some(2.0 * std::f64::consts::PI));
        assert_eq!(Expr::parse("2 * t").unwrap().constant(), None);
    }
}
