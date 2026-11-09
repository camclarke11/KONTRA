//! Practice ("wait") mode as the editor shows it: the hand chosen in the
//! header, and while a song waits for the player, the keys to press marked
//! on the falling notes and on the keybed, those of them struck, and wrong
//! keys held. The host's gate decides; this only draws what it says.

use mui::prelude::*;

use crate::keyboard::{black, layout};
use crate::kit;

/// Whose notes the player plays while the song waits for them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Hands {
    /// The song plays through, waiting for nobody.
    #[default]
    Off,
    /// The song's highest voice (a track and channel) on average, or its
    /// notes from middle C up when it has one voice.
    Right,
    /// Its lowest voice, or its notes below middle C.
    Left,
    /// Every note.
    Both,
}

impl Hands {
    /// The choices, in the order the header lists them.
    pub const ALL: [Self; 4] = [Self::Off, Self::Right, Self::Left, Self::Both];

    /// As the header names it.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Right => "Right hand",
            Self::Left => "Left hand",
            Self::Both => "Both hands",
        }
    }
}

/// What the practice gate shows the editor.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Practice {
    pub hands: Hands,
    /// The song stands at the keys in `want`, waiting for them.
    pub waiting: bool,
    /// The keys of the chord the song waits for next.
    pub want: Vec<u8>,
    /// Those of them struck so far.
    pub struck: Vec<u8>,
    /// Keys held that the song does not want now.
    pub wrong: Vec<u8>,
}

impl Practice {
    /// Whether there is anything to mark on the keys.
    #[must_use]
    pub fn marks(&self) -> bool {
        self.hands != Hands::Off && !(self.want.is_empty() && self.wrong.is_empty())
    }
}

/// A key to press: light, so it reads on any note's colour.
fn wanted() -> Color {
    Color::oklch(0.97, 0., 0.)
}

/// A key of the chord already struck: green, for done.
fn struck() -> Color {
    Color::oklch(0.78, 0.16, 150.)
}

/// A wrong key: red, short of the coral the editor keeps for load errors.
fn wrong() -> Color {
    Color::oklch(0.62, 0.21, 22.)
}

/// The header's "Practice" dropdown; the hand picked, if any.
pub fn control(
    ui: &mut Ui,
    open: &mut Option<String>,
    practice: &Practice,
) -> Response<Option<Hands>> {
    let labels = Hands::ALL.map(Hands::label);
    let current = Hands::ALL.iter().position(|h| *h == practice.hands);
    let label = match (practice.hands, practice.waiting) {
        (Hands::Off, _) => "Practice".to_owned(),
        (h, true) => format!("{} · waiting", h.label()),
        (h, false) => h.label().to_owned(),
    };
    let tip = "Practice: the song waits at each of your notes until you play them";
    let r = kit::dropdown(ui, open, "practice", (None, &label, tip), &labels, current);
    Response {
        el: r.el,
        changed: r.changed.and_then(|n| Hands::ALL.get(n).copied()),
    }
}

/// A circle of radius `r` about `c`, from four cubic arcs.
fn circle(c: Point, r: f64) -> Path {
    // The control distance that makes a quarter circle of a cubic.
    let k = r * 0.552_284_75;
    let p = |x: f64, y: f64| Point::new(c.x + x, c.y + y);
    Path::default()
        .move_to(p(r, 0.))
        .cubic_to(p(r, k), p(k, r), p(0., r))
        .cubic_to(p(-k, r), p(-r, k), p(-r, 0.))
        .cubic_to(p(-r, -k), p(-k, -r), p(0., -r))
        .cubic_to(p(k, -r), p(r, -k), p(r, 0.))
        .close()
}

fn rect(x: f64, y: f64, w: f64, h: f64) -> Path {
    Path::polyline(
        [(x, y), (x + w, y), (x + w, y + h), (x, y + h)].map(|(x, y)| Point::new(x, y)),
        true,
    )
}

/// What the lane over the keys draws on a canvas of `size`: a target on
/// the keys' edge for each key to press (filled once struck), a red bar
/// rising from it for each wrong key held, and while the song waits, a
/// line along the edge where it stands.
fn lane_draw(p: &Practice, range: (u8, u8), size: Size) -> Vec<Draw> {
    let (w, h) = (size.width, size.height);
    let keys = layout(range, w);
    let place = |key: u8| {
        let n = key.checked_sub(range.0)?;
        (key <= range.1).then(|| keys.get(usize::from(n)).copied())?
    };
    let mut out = Vec::new();
    if p.waiting {
        out.push(Draw::fill(
            rect(0., h - 2., w, 2.),
            Fill::from(wanted().with_alpha(0.5)),
        ));
    }
    let tall = (h * 0.06).clamp(6., 14.);
    for &key in &p.want {
        let Some((_, x, kw)) = place(key) else {
            continue;
        };
        let inset = kw * if black(key) { 0.08 } else { 0.07 };
        let (x, kw) = (x + inset, kw - 2. * inset);
        let done = p.struck.contains(&key);
        let c = if done { struck() } else { wanted() };
        let r = rect(x + 1., h - tall, kw - 2., tall - 1.);
        if done {
            out.push(Draw::fill(r, Fill::from(c.with_alpha(0.85))));
        } else {
            out.push(Draw::stroke(r, Fill::from(c), 2.));
        }
    }
    for &key in &p.wrong {
        let Some((_, x, kw)) = place(key) else {
            continue;
        };
        out.push(Draw::fill(
            rect(x + 1., h - tall * 2.5, kw - 2., tall * 2.5),
            Gradient::linear(
                180.,
                [(0., wrong().with_alpha(0.)), (1., wrong().with_alpha(0.9))],
            ),
        ));
    }
    out
}

/// The practice marks over the falling notes of a keybed showing `range`,
/// `height` tall: laid in a stack over the lane.
#[must_use]
pub fn lane(practice: &Practice, range: (u8, u8), height: f64) -> El {
    let p = practice.clone();
    canvas(move |s| lane_draw(&p, range, s))
        .w(Len::Pct(100.))
        .h(height)
        .named("Practice marks")
        .id("practice-lane")
}

/// What the keybed's marks draw on a canvas of `size`: a dot on each key to
/// press (green once struck) and wrong keys washed red.
fn keys_draw(p: &Practice, range: (u8, u8), size: Size) -> Vec<Draw> {
    let (w, h) = (size.width, size.height);
    let keys = layout(range, w);
    let place = |key: u8| {
        let n = key.checked_sub(range.0)?;
        (key <= range.1).then(|| keys.get(usize::from(n)).copied())?
    };
    let mut out = Vec::new();
    for &key in &p.wrong {
        let Some((_, x, kw)) = place(key) else {
            continue;
        };
        let tall = if black(key) { h * 0.62 } else { h };
        out.push(Draw::fill(
            rect(x + 0.5, 0., kw - 1., tall),
            Fill::from(wrong().with_alpha(0.75)),
        ));
    }
    for &key in &p.want {
        let Some((_, x, kw)) = place(key) else {
            continue;
        };
        let done = p.struck.contains(&key);
        let r = (kw * 0.28).clamp(2., 7.);
        let cy = if black(key) { h * 0.45 } else { h * 0.84 };
        let dot = circle(Point::new(x + kw / 2., cy), r);
        let ring = Color::oklch(0.15, 0., 0.);
        out.push(Draw::fill(
            dot.clone(),
            Fill::from(if done { struck() } else { wanted() }),
        ));
        out.push(Draw::stroke(dot, Fill::from(ring.with_alpha(0.8)), 1.));
    }
    out
}

/// The practice marks over a keybed showing `range`, `height` tall: laid
/// in a stack over the keys; it takes no pointer, so the keys still play.
#[must_use]
pub fn keys(practice: &Practice, range: (u8, u8), height: f64) -> El {
    let p = practice.clone();
    canvas(move |s| keys_draw(&p, range, s))
        .w(Len::Pct(100.))
        .h(height)
        .named("Practice keys")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn practice() -> Practice {
        Practice {
            hands: Hands::Right,
            waiting: true,
            want: vec![60, 64, 67],
            struck: vec![64],
            wrong: vec![62],
        }
    }

    #[test]
    fn the_marks_show_only_while_practising() {
        assert!(practice().marks());
        let off = Practice {
            hands: Hands::Off,
            ..practice()
        };
        assert!(!off.marks());
        assert!(!Practice::default().marks());
    }

    #[test]
    fn each_key_to_press_and_each_wrong_key_is_marked() {
        let p = practice();
        let size = Size::new(700., 200.);
        // A line for waiting, three targets, one wrong bar.
        assert_eq!(lane_draw(&p, (48, 83), size).len(), 1 + 3 + 1);
        // Two draws a dot (face and ring), one wash.
        assert_eq!(keys_draw(&p, (48, 83), size).len(), 2 * 3 + 1);
        // Keys off the keybed draw nothing.
        assert_eq!(keys_draw(&p, (72, 83), size).len(), 0);
        assert!(lane_draw(&Practice::default(), (48, 83), size).is_empty());
        assert_eq!(Hands::ALL.map(Hands::label)[2], "Left hand");
    }
}
