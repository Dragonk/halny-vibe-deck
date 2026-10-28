//! The Halny brand mark — **exactly 1:1 from the stable Electron edition**
//! (`src/renderer/src/components/BrandIcon.tsx` / `resources/halny-icon.svg`):
//! a rounded-square `#10131A` tile, a crisp-edge pixel pyramid in the Halny
//! oranges, and two wind-swoosh paths. Used in the header brand, the setup
//! screen and the chat empty state. Do not restyle — the owner requires
//! pixel parity with the stable edition.

use dioxus::prelude::*;

/// The 26 unit squares of the pyramid, in SVG space (UNIT = 60, viewBox 1024).
const PYRAMID: &[(i32, i32, &str)] = &[
    (272, 732, "#E85D04"),
    (272, 662, "#E85D04"),
    (342, 732, "#E85D04"),
    (342, 662, "#E85D04"),
    (342, 592, "#FF8205"),
    (342, 522, "#FF8205"),
    (412, 732, "#E85D04"),
    (412, 662, "#E85D04"),
    (412, 592, "#FF8205"),
    (412, 522, "#FF8205"),
    (412, 452, "#FFB020"),
    (412, 382, "#FFD800"),
    (482, 732, "#E85D04"),
    (482, 662, "#E85D04"),
    (482, 592, "#FF8205"),
    (552, 732, "#E85D04"),
    (552, 662, "#E85D04"),
    (552, 592, "#FF8205"),
    (552, 522, "#FF8205"),
    (552, 452, "#FFB020"),
    (552, 382, "#FFD800"),
    (622, 732, "#E85D04"),
    (622, 662, "#E85D04"),
    (622, 592, "#FF8205"),
    (622, 522, "#FF8205"),
    (692, 732, "#E85D04"),
];

#[component]
pub fn BrandMark(size: f32, class: String) -> Element {
    let w = size as u32;
    let h = size as u32;
    rsx! {
        svg {
            class: "{class}",
            xmlns: "http://www.w3.org/2000/svg",
            width: "{w}", height: "{h}",
            view_box: "0 0 1024 1024",
            rect { x: "0", y: "0", width: "1024", height: "1024", rx: "220", fill: "#10131A" }
            g { shape_rendering: "crispEdges",
                for (x, y, fill) in PYRAMID.iter() {
                    rect { x: "{x}", y: "{y}", width: "60", height: "60", fill: "{fill}" }
                }
            }
            path { d: "M 258 330 Q 512 262 766 290", fill: "none", stroke: "#F2F5FA", stroke_width: "26", stroke_linecap: "round" }
            path { d: "M 272 358 Q 512 298 750 318", fill: "none", stroke: "#FFD800", stroke_width: "11", stroke_linecap: "round" }
        }
    }
}
