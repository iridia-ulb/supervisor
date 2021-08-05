use wasm_bindgen::prelude::*;
use yew::web_sys::HtmlElement;

pub mod card;
pub mod button;
pub mod grid;
pub mod layout;
pub mod tabs;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = componentHandler, js_name = upgradeElement)]
    fn upgrade_element(element: HtmlElement);
}

// #[wasm_bindgen]
// extern "C" {
//     #[wasm_bindgen(js_namespace = console, js_name = log)]
//     fn log_text(string: &str);

//     #[wasm_bindgen(js_namespace = console, js_name = log)]
//     fn log_element(element: &HtmlElement);
// }