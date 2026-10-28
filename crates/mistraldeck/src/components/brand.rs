//! The Halny brand mark — Tatra ridge with foehn-wind gusts, the
//! same geometry as the app icon (viewBox 96×70). Used in the
//! header brand and the chat empty state.

use dioxus::prelude::*;

#[component]
pub fn BrandMark(size: f32, class: String) -> Element {
    let w = (96.0 * size) as u32;
    let h = (70.0 * size) as u32;
    rsx! {
        svg {
            class: "{class}",
            xmlns: "http://www.w3.org/2000/svg",
            width: "{w}", height: "{h}",
            view_box: "0 0 96 70",
            path { d: "M14 12 H46 Q54 12 54 7", stroke: "var(--yellowbright-500)", stroke_width: "5", fill: "none", stroke_linecap: "round" }
            path { d: "M6 24 H60 Q70 24 70 17", stroke: "var(--orangebright-500)", stroke_width: "5", fill: "none", stroke_linecap: "round" }
            path { d: "M18 36 H48", stroke: "var(--orange-500)", stroke_width: "5", fill: "none", stroke_linecap: "round" }
            path { d: "M34 66 L62 24 L94 66 Z", fill: "var(--red-700)", opacity: "0.85" }
            path { d: "M2 66 L30 20 L58 66 Z", fill: "var(--accent)" }
            path { d: "M30 20 L35 28.5 H25 Z", fill: "#ffffff", opacity: "0.92" }
            path { d: "M62 24 L66.5 31.5 H57.5 Z", fill: "#ffffff", opacity: "0.92" }
        }
    }
}
