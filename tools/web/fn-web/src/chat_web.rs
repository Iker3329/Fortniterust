use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

fn page_fn(name: &str) -> Option<js_sys::Function> { js_sys::Reflect::get(&js_sys::global(), &name.into()).ok().and_then(|f| f.dyn_into::<js_sys::Function>().ok()) }

#[wasm_bindgen]
pub fn fnweb_chat_text(text: String) { super::push_inbound(text); }

pub fn send_text(json: &str) -> bool {
    page_fn("__fnweb_mp_text_send").and_then(|f| f.call1(&JsValue::NULL, &json.into()).ok()).and_then(|v| v.as_bool()).unwrap_or(false)
}

pub fn lobby_code() -> Option<String> {
    page_fn("__fnweb_mp_code").and_then(|f| f.call0(&JsValue::NULL).ok()).and_then(|v| v.as_string()).filter(|c| !c.is_empty())
}

pub fn join(code: &str, name: &str) {
    if let Some(f) = page_fn("__fnweb_mp_join") { let _ = f.call2(&JsValue::NULL, &code.into(), &name.into()); }
}

pub fn leave() {
    if let Some(f) = page_fn("__fnweb_mp_leave") { let _ = f.call0(&JsValue::NULL); }
}
