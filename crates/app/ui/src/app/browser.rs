//! The browser, as v2 laid it out: the libraries above (search, order,
//! filter, format, favorites, recent), the chosen library's instruments
//! below as a folder tree. Its one action is to pick an instrument.

use mui::prelude::*;

use std::sync::Arc;
use std::time::{Duration, Instant};

use kontra_ui_model::{AssetRef, ImageMeta};

use super::{Action, App, Format, Shelf, View};
use crate::Pictures;
use crate::kit::{self, Icon, field, glyph, icon_button, interactive, list_row, rule, tab};
use crate::theme::{CONTROL, SIDEBAR, SMALL, SPACE, TEXT, TIGHT, accent, secondary};

pub(super) fn view(ui: &mut Ui, app: &mut App, out: &mut Vec<Action>) -> El {
    if app.view.format == Format::Songs {
        return super::songs::view(ui, app, out);
    }
    let top = libraries(ui, app, out);
    let bottom = presets(ui, app, out);
    let edge = split(ui, app);
    // The libraries' share of the column as it was laid out last frame: a
    // share of free space would leave them none, the presets' rows being
    // longer than the column.
    let top = match column_height(ui) {
        Some(h) => top.h(h * shared(app.view.split)).shrink(0),
        None => top.grow(1).min_h(0),
    };
    col![top, edge, bottom.grow(1).min_h(0)]
        .h(Len::Pct(100.))
        .id("browser-column")
}

/// The library list's share of the browser's height, as v2 split it; a
/// split never set (0) is v2's default.
const SPLIT: f64 = 0.55;
const SPLIT_MIN: f64 = 0.14;
const SPLIT_MAX: f64 = 0.7;

fn column_height(ui: &Ui) -> Option<f64> {
    (ui.scene())
        .and_then(|s| s.surface("browser-column"))
        .map(|s| s.frame.size.height)
        .filter(|&h| h > 0.)
}

fn shared(split: f64) -> f64 {
    if split > 0. {
        split.clamp(SPLIT_MIN, SPLIT_MAX)
    } else {
        SPLIT
    }
}

/// The edge between the libraries and the presets (v2's `src/ui/browser.rs`
/// `split_divider`): a drag shares out the height, a double click resets
/// it. A hairline at rest, lit under the hand.
fn split(ui: &mut Ui, app: &mut App) -> El {
    let id = "browser-split";
    let r = ui.get(id);
    let height = column_height(ui).unwrap_or(600.);
    if r.dragged {
        let now = shared(app.view.split) + r.drag_delta.y / height.max(1.);
        app.view.split = now.clamp(SPLIT_MIN, SPLIT_MAX);
    }
    if r.double_clicked {
        app.view.split = 0.;
    }
    let lit = r.hovered || r.held;
    col![block(Len::Pct(100.), 1).fill(if lit {
        Fill::from(accent())
    } else {
        Role::Ink.alpha(0.08)
    })]
    .justify(Justify::Center)
    .w(Len::Pct(100.))
    .h(2. * TIGHT + 1.)
    .shrink(0)
    .cursor(Cursor::ResizeV)
    .tip("Drag to share out the height, double-click to reset")
    .named("Resize the library list")
    .id(id)
}

fn matches(name: &str, search: &str) -> bool {
    search.is_empty() || name.to_lowercase().contains(&search.to_lowercase())
}

fn libraries(ui: &mut Ui, app: &mut App, out: &mut Vec<Action>) -> El {
    let v = &mut app.view;
    let search = field(ui, "search", "Search", &mut v.search);
    let order = kit::dropdown(
        ui,
        &mut v.menu,
        "sort",
        (None, if v.descending { "Z–A" } else { "A–Z" }, "Order"),
        &["A–Z", "Z–A"],
        Some(usize::from(v.descending)),
    );
    if let Some(o) = order.changed {
        v.descending = o == 1;
    }
    let add = icon_button(ui, "browser-add", Icon::Plus, "Add a library folder", false);
    if add.changed {
        out.push(Action::AddFolder);
    }
    let hide = icon_button(ui, "browser-hide", Icon::Left, "Hide the browser", false);
    if hide.changed {
        v.browser_hidden = true;
    }
    let libraries = &app.libraries;
    let formats = formats(
        ui,
        v,
        |f| libraries.iter().filter(|l| l.format == f).count(),
        out,
    );
    let starred = app.libraries.iter().filter(|l| l.starred).count();
    let shelf_row = shelves(
        ui,
        "shelf",
        [
            (starred, app.view.shelf == Shelf::Favorites),
            (app.recent.len(), app.view.shelf == Shelf::Recent),
        ],
    );
    if let Some(picked) = shelf_row.changed {
        let shelf = [Shelf::Favorites, Shelf::Recent]
            .get(picked)
            .copied()
            .unwrap_or_default();
        app.view.shelf = if app.view.shelf == shelf {
            Shelf::All
        } else {
            shelf
        };
    }
    let list = if app.view.shelf == Shelf::Recent {
        recent(ui, app, out)
    } else {
        library_list(ui, app, out)
    };
    col![
        row![search.el.grow(1).min_w(0), order.el, add.el, hide.el]
            .gap(TIGHT)
            .align(Align::Center)
            .pad(kit::edges(SPACE, SPACE, 0., SPACE)),
        formats,
        rule(),
        shelf_row.el,
        rule(),
        list.grow(1).min_h(0),
    ]
}

/// Favorites and Recent side by side, each with how many it holds, `id`
/// and the label naming each: a click on one narrows the list to it (the
/// caller turns a second click back to all). Which was clicked.
pub(super) fn shelves(
    ui: &mut Ui,
    id: &str,
    [(starred, favorites), (played, recent)]: [(usize, bool); 2],
) -> Response<Option<usize>> {
    let mut picked = None;
    let cells: Vec<El> = [
        (Icon::Star, "Favorites", starred, favorites),
        (Icon::Clock, "Recent", played, recent),
    ]
    .into_iter()
    .enumerate()
    .map(|(n, (icon, label, count, on))| {
        let id = format!("{id}-{label}");
        if ui.get(id.clone()).activated() {
            picked = Some(n);
        }
        let ink = if on {
            Fill::from(Role::Ink)
        } else {
            secondary()
        };
        interactive(
            row![
                glyph(icon, TEXT, ink.clone()),
                body(label).text_size(TEXT).fill(ink).lines(1).grow(1),
                caption(count.to_string()).fill(secondary()),
            ]
            .gap(TIGHT * 1.5)
            .align(Align::Center)
            .h(CONTROL)
            .pad((TIGHT * 2., 0.))
            .grow(1)
            .w(0.)
            .fill(if on { Role::Raised } else { Role::Field }),
        )
        .focusable()
        .a11y(A11y::Toggle { on })
        .named(label)
        .tip(format!("Show only {}; again for all", label.to_lowercase()))
        .id(id)
    })
    .collect();
    Response {
        el: row(cells).gap(TIGHT).pad(TIGHT).shrink(0),
        changed: picked,
    }
}

/// The browser's tabs, Kontakt, Falcon / UVI and Songs, and how many
/// `count` says the one shown lists. Choosing Songs reads its folders again.
pub(super) fn formats(
    ui: &mut Ui,
    v: &mut View,
    count: impl Fn(Format) -> usize,
    out: &mut Vec<Action>,
) -> El {
    let mut tabs = Vec::new();
    for (format, label) in [
        (Format::Kontakt, "Kontakt"),
        (Format::Uvi, "Falcon / UVI"),
        (Format::Songs, "Songs"),
    ] {
        let t = tab(ui, format!("format-{label}"), label, v.format == format);
        if t.changed && v.format != format {
            (v.format, v.library) = (format, None);
            v.search.clear();
            if format == Format::Songs {
                out.push(Action::Songs(super::SongAction::Rescan));
            }
        }
        tabs.push(t.el);
    }
    tabs.extend([
        spacer(),
        caption(count(v.format).to_string()).fill(secondary()),
    ]);
    row(tabs)
        .align(Align::Center)
        .pad(kit::edges(0., SPACE, 0., 0.))
}

/// Why a list shows nothing, and what to do about it: `lines` dim and
/// centred, then an action when there is one.
pub(super) fn empty(lines: &[&str], action: Option<El>) -> El {
    let mut items: Vec<El> = (lines.iter())
        .map(|l| {
            caption((*l).to_owned())
                .fill(secondary())
                .lines(3)
                .max_size((SIDEBAR - 4. * SPACE, TEXT * 4.))
        })
        .collect();
    items.extend(action);
    col(items)
        .gap(SPACE)
        .align(Align::Center)
        .pad(SPACE * 2.)
        .w(Len::Pct(100.))
}

/// The library list's empty state: nothing found yet, nothing of this
/// format, no favorites, or no match.
fn no_libraries(ui: &mut Ui, app: &App, out: &mut Vec<Action>) -> El {
    let v = &app.view;
    let add = |ui: &mut Ui, out: &mut Vec<Action>| {
        let r = kit::button(ui, "browser-empty-add", "Add a folder…", false);
        if r.changed {
            out.push(Action::AddFolder);
        }
        r.el.fill(Role::Raised)
            .tip("Pick the folder your libraries are in; KONTRA finds them inside it")
    };
    let format = if v.format == Format::Uvi {
        "Falcon / UVI"
    } else {
        "Kontakt"
    };
    if !v.search.is_empty() {
        return empty(&["Nothing matches"], None);
    }
    if app.libraries.is_empty() {
        let lines: &[&str] = if app.folders.is_empty() {
            &["No libraries yet", "Add the folder your libraries are in"]
        } else {
            &["No libraries found in your folders yet"]
        };
        return empty(lines, Some(add(ui, out)));
    }
    if v.shelf == Shelf::Favorites {
        return empty(&["No favorites yet: star a library to keep it here"], None);
    }
    let none = format!("No {format} libraries in your folders");
    empty(&[&none], Some(add(ui, out)))
}

fn library_list(ui: &mut Ui, app: &mut App, out: &mut Vec<Action>) -> El {
    let v = &app.view;
    let mut shown: Vec<(usize, &super::Library)> = app
        .libraries
        .iter()
        .enumerate()
        .filter(|(_, l)| {
            l.format == v.format
                && (l.starred || v.shelf != Shelf::Favorites)
                && matches(&l.name, &v.search)
        })
        .collect();
    shown.sort_by_cached_key(|(_, l)| l.name.to_lowercase());
    if v.descending {
        shown.reverse();
    }
    if shown.is_empty() {
        return no_libraries(ui, app, out);
    }
    let mut picked = None;
    let mut covers = app.covers.as_mut();
    let rows: Vec<El> = shown
        .iter()
        .map(|&(n, l)| {
            let id = format!("library-{n}");
            if ui.get(id.as_str()).activated() {
                picked = Some(n);
            }
            let image = covers.as_deref_mut().and_then(|c| cover(c, n, &l.art));
            card(id, &l.name, l.presets.len(), image, v.library == Some(n))
        })
        .collect();
    if let Some(n) = picked {
        app.view.library = Some(n);
        app.view.preset_search.clear();
    }
    col(rows).gap(1).pad(TIGHT).scroll().min_h(0)
}

/// Widest official browser banner (Afflatus Chapter II Brass, 905x99):
/// every library card has its shape (v2's).
const BANNER_RATIO: f64 = 905. / 99.;

/// A library as v2's browser drew it (`src/ui/browser.rs` `source_row`,
/// the card): its banner across the sidebar under a dark scrim, the name
/// and preset count in white over it, an accent edge when chosen.
fn card(id: String, name: &str, count: usize, image: Option<Arc<Image>>, chosen: bool) -> El {
    let height = (SIDEBAR - 2. * TIGHT) / BANNER_RATIO;
    let art = match image {
        Some(image) => block(Len::Pct(100.), height).fill(Fill::Image(image, Fit::Cover)),
        None => block(Len::Pct(100.), height).fill(Role::Raised),
    };
    let scrim = block(Len::Pct(100.), Len::Pct(100.)).fill(Gradient::linear(
        90.,
        [
            (0., Color::srgb(0., 0., 0.).with_alpha(0.88)),
            (0.65, Color::srgb(0., 0., 0.).with_alpha(0.72)),
            (1., Color::srgb(0., 0., 0.).with_alpha(0.72)),
        ],
    ));
    let mark = block(2, height).fill(if chosen {
        Fill::from(accent())
    } else {
        Role::Ink.alpha(0.)
    });
    let label = row![
        mark,
        body(name.to_owned())
            .text_size(TEXT)
            .fill(Color::srgb(1., 1., 1.))
            .lines(1)
            .min_w(0)
            .grow(1),
        caption(count.to_string())
            .text_size(SMALL * 0.85)
            .fill(Color::srgb(0.9, 0.9, 0.9))
            .shrink(0),
    ]
    .gap(TIGHT)
    .align(Align::Center)
    .pad((SPACE, 0.))
    .h(Len::Pct(100.));
    let el = col![stack![art, scrim, label].w(Len::Pct(100.)).h(height).clip()]
        .pad((TIGHT, TIGHT))
        .w(Len::Pct(100.))
        .when(chosen, |e| e.fill(Role::Raised))
        .focusable()
        .a11y(A11y::Button)
        .named(format!("{name}, {count} presets"))
        .tip(format!("{name}\nShow its presets below"))
        .id(id)
        .shrink(0);
    kit::interactive(el)
}

fn recent(ui: &mut Ui, app: &mut App, out: &mut Vec<Action>) -> El {
    let view = &mut app.view;
    let rows: Vec<El> = app
        .recent
        .iter()
        .enumerate()
        .filter_map(|(n, &(library, preset))| {
            let l = app.libraries.get(library)?;
            let name = l.presets.get(preset)?;
            let id = format!("recent-{n}");
            let at = (library, preset);
            let r = list_row(
                ui,
                id.clone(),
                (name, &l.name),
                None,
                view.picked == Some(at),
            );
            pick(ui, &id, (&mut view.picked, &mut view.clicked), at, out);
            Some(r.el)
        })
        .collect();
    if rows.is_empty() {
        return empty(&["Nothing loaded yet"], None);
    }
    col(rows).gap(1).pad(TIGHT).scroll().min_h(0)
}

/// The chosen library's instruments, foldered by the path inside it.
fn presets(ui: &mut Ui, app: &mut App, out: &mut Vec<Action>) -> El {
    let loading = app.loading.is_some();
    let v = &mut app.view;
    let search = field(
        ui,
        "preset-search",
        "Search presets and folders",
        &mut v.preset_search,
    );
    let Some((lib, l)) = v
        .library
        .and_then(|n| app.libraries.get_mut(n).map(|l| (n, l)))
    else {
        let hint = (!app.libraries.is_empty())
            .then(|| empty(&["Pick a library above to see its instruments"], None));
        return col![search.el, col(hint)];
    };
    let cover = (app.covers.as_mut()).and_then(|c| cover(c, lib, &l.art));
    let star = icon_button(ui, "star", Icon::Star, "Favorite", l.starred);
    if star.changed {
        l.starred = !l.starred;
        out.push(Action::Star(lib, l.starred));
    }
    let title = col![
        body(l.name.clone())
            .text_weight(Weight::SEMIBOLD)
            .lines(1)
            .min_w(0)
            .tip(l.name.clone()),
        caption(format!("{} instruments", l.presets.len())).fill(secondary()),
    ]
    .gap(1)
    .align(Align::Start)
    .grow(1)
    .min_w(0);
    let loaded: Vec<&str> = app.parts.iter().map(|p| p.name.as_str()).collect();
    let mut folders: Vec<(&str, Vec<(usize, &str)>)> = Vec::new();
    for (n, path) in l.presets.iter().enumerate() {
        if !matches(path, &v.preset_search) {
            continue;
        }
        let (folder, name) = path.rsplit_once('/').unwrap_or(("Instruments", path));
        match folders.iter_mut().find(|f| f.0 == folder) {
            Some(f) => f.1.push((n, name)),
            None => folders.push((folder, vec![(n, name)])),
        }
    }
    let mut rows = Vec::new();
    let mut fold = None;
    for (folder, items) in &folders {
        let id = format!("folder-{folder}");
        let shut = v.folded.iter().any(|f| f == folder);
        if ui.get(id.clone()).activated() {
            fold = Some((*folder).to_owned());
        }
        rows.push(
            interactive(
                row![
                    glyph(
                        if shut { Icon::Right } else { Icon::Down },
                        TEXT * 0.8,
                        secondary()
                    ),
                    body((*folder).to_owned())
                        .text_size(TEXT)
                        .lines(1)
                        .min_w(0)
                        .grow(1),
                    caption(items.len().to_string()).fill(secondary()),
                ]
                .gap(TIGHT)
                .align(Align::Center)
                .h(CONTROL)
                .pad((0., SPACE)),
            )
            .focusable()
            .a11y(A11y::Button)
            .named((*folder).to_owned())
            .id(id),
        );
        if shut {
            continue;
        }
        for &(n, name) in items {
            let id = format!("preset-{n}");
            let at = (lib, n);
            let shown = loaded.contains(&name) || v.picked == Some(at);
            let busy = loading && v.picked == Some(at);
            let r = list_row(
                ui,
                id.clone(),
                (name, if busy { "Loading…" } else { "" }),
                None,
                shown,
            );
            pick(ui, &id, (&mut v.picked, &mut v.clicked), at, out);
            rows.push(r.el.pad(kit::edges(0., 0., 0., SPACE)));
        }
    }
    if let Some(f) = fold {
        match v.folded.iter().position(|x| *x == f) {
            Some(at) => {
                v.folded.remove(at);
            }
            None => v.folded.push(f),
        }
    }
    let banner = cover.map(|img| {
        block(Len::Pct(100.), Len::Auto)
            .aspect(BANNER)
            .fill(Fill::Image(img, Fit::Cover))
            .pad((0., SPACE))
    });
    col![
        search.el,
        col(banner),
        row![title, star.el]
            .gap(TIGHT)
            .align(Align::Center)
            .pad((TIGHT, SPACE)),
        col(rows)
            .pad(kit::edges(0., SPACE, 0., 0.))
            .scroll()
            .min_h(0)
            .grow(1),
    ]
}

/// A preset on its way from the browser to the rack.
pub(super) struct Carried {
    pub library: usize,
    pub preset: usize,
}

/// mui's double-click window.
const DOUBLE_CLICK: Duration = Duration::from_millis(400);

/// A preset row's pointer, as v2 read it: a click selects it, a
/// double-click or Enter loads it into a new part, a drag carries it to
/// the rack (onto a part to replace it, onto free space to add one).
fn pick(
    ui: &mut Ui,
    id: &str,
    (picked, clicked): (&mut Option<(usize, usize)>, &mut Option<Instant>),
    (library, preset): (usize, usize),
    out: &mut Vec<Action>,
) {
    let r = ui.get(id);
    // The second click on the selected row is the double-click: mui's own
    // flag lives for one input intake, which a real host's release clears
    // before the next build reads it.
    let again = r.clicked
        && *picked == Some((library, preset))
        && clicked.is_some_and(|t| t.elapsed() < DOUBLE_CLICK);
    if again || r.key_activated {
        *clicked = None;
        out.push(Action::Load { library, preset });
    } else if r.clicked {
        *picked = Some((library, preset));
        *clicked = Some(Instant::now());
    }
    if r.dragged && ui.dragging::<Carried>().is_none() {
        *picked = Some((library, preset));
        ui.start_drag(id, Carried { library, preset });
    }
}

/// A library-browser banner's width over its height (Kontakt's 905 × 99).
const BANNER: f64 = 905. / 99.;
/// The most cover pictures of one library tried.
const COVERS: usize = 4;

/// The first of a library's cover pictures that has decoded; asks for the
/// rest meanwhile.
pub(super) fn cover(covers: &mut Pictures, lib: usize, art: &[String]) -> Option<Arc<Image>> {
    let meta = ImageMeta::default();
    (art.iter().take(COVERS).enumerate())
        .find_map(|(k, path)| covers.frame(AssetRef(lib * COVERS + k), path, meta, 0, 1.))
}
