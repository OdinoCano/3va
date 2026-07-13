use serde::Serialize;
use std::collections::HashMap;
use std::io::BufRead;
use std::sync::{Arc, Mutex};
use v8::{FunctionCallbackArguments, PinScope, ReturnValue};

type EventQueue = Arc<Mutex<Vec<SseEvent>>>;
type EsMap = Arc<Mutex<HashMap<u32, EsEntry>>>;

struct EsEntry {
    events: EventQueue,
    cancel: Arc<Mutex<bool>>,
}

#[derive(Clone, Serialize)]
struct SseEvent {
    #[serde(rename = "type")]
    event_type: String,
    data: String,
    #[serde(rename = "lastEventId")]
    last_event_id: String,
}

// Each JsEngine/V8 isolate stays pinned to the OS thread that created it for
// its whole lifetime, so a thread_local map (rather than a process-wide
// OnceLock) keeps parallel engines/tests from sharing EventSource state.
thread_local! {
    static ES_MAP: EsMap = Arc::new(Mutex::new(HashMap::new()));
    static ES_COUNTER: Arc<Mutex<u32>> = Arc::new(Mutex::new(0u32));
}
fn map() -> EsMap {
    ES_MAP.with(|m| m.clone())
}
fn counter() -> Arc<Mutex<u32>> {
    ES_COUNTER.with(|c| c.clone())
}

pub fn inject_event_source(scope: &mut v8::ContextScope<v8::HandleScope>) {
    let context = scope.get_current_context();
    let global = context.global(scope);

    let open_fn = v8::Function::new(
        scope,
        move |scope: &mut PinScope<'_, '_>,
              args: FunctionCallbackArguments,
              mut rv: ReturnValue| {
            let url = args.get(0).to_rust_string_lossy(scope);

            let counter_arc = counter();
            let mut id_lock = counter_arc.lock().unwrap();
            *id_lock += 1;
            let id = *id_lock;
            drop(id_lock);

            let queue: EventQueue = Arc::new(Mutex::new(Vec::new()));
            let cancel = Arc::new(Mutex::new(false));

            let q2 = queue.clone();
            let c2 = cancel.clone();
            let url2 = url.clone();

            std::thread::spawn(move || {
                let resp = match ureq::get(&url2)
                    .set("Accept", "text/event-stream")
                    .set("Cache-Control", "no-cache")
                    .call()
                {
                    Ok(r) => r,
                    Err(e) => {
                        q2.lock().unwrap().push(SseEvent {
                            event_type: "error".to_string(),
                            data: e.to_string(),
                            last_event_id: String::new(),
                        });
                        return;
                    }
                };

                let reader = std::io::BufReader::new(resp.into_reader());
                let mut data_buf = String::new();
                let mut event_type = "message".to_string();
                let mut last_id = String::new();

                for line in reader.lines() {
                    if *c2.lock().unwrap() {
                        break;
                    }
                    let line = match line {
                        Ok(l) => l,
                        Err(_) => break,
                    };

                    if line.is_empty() {
                        if !data_buf.is_empty() {
                            let data = if data_buf.ends_with('\n') {
                                data_buf[..data_buf.len() - 1].to_string()
                            } else {
                                data_buf.clone()
                            };
                            q2.lock().unwrap().push(SseEvent {
                                event_type: event_type.clone(),
                                data,
                                last_event_id: last_id.clone(),
                            });
                        }
                        data_buf.clear();
                        event_type = "message".to_string();
                    } else if let Some(val) = line.strip_prefix("data:") {
                        let val = val.strip_prefix(' ').unwrap_or(val);
                        data_buf.push_str(val);
                        data_buf.push('\n');
                    } else if let Some(val) = line.strip_prefix("event:") {
                        event_type = val.strip_prefix(' ').unwrap_or(val).to_string();
                    } else if let Some(val) = line.strip_prefix("id:") {
                        last_id = val.strip_prefix(' ').unwrap_or(val).to_string();
                    }
                }
            });

            let map_arc = map();
            map_arc.lock().unwrap().insert(
                id,
                EsEntry {
                    events: queue,
                    cancel,
                },
            );

            rv.set(v8::Number::new(scope, id as f64).into());
        },
    )
    .unwrap();
    global.set(
        scope,
        v8::String::new(scope, "__eventSourceOpen").unwrap().into(),
        open_fn.into(),
    );

    let poll_fn = v8::Function::new(
        scope,
        move |scope: &mut PinScope<'_, '_>,
              args: FunctionCallbackArguments,
              mut rv: ReturnValue| {
            let id = args.get(0).uint32_value(scope).unwrap_or(0);

            let map_arc = map();
            let m = map_arc.lock().unwrap();
            let entry = match m.get(&id) {
                Some(e) => e,
                None => {
                    rv.set(v8::null(scope).into());
                    return;
                }
            };

            let mut q = entry.events.lock().unwrap();
            if q.is_empty() {
                rv.set(v8::String::new(scope, "[]").unwrap().into());
                return;
            }

            let drained: Vec<SseEvent> = q.drain(..).collect();
            drop(q);

            let json = serde_json::to_string(&drained).unwrap_or_else(|_| "[]".to_string());
            rv.set(v8::String::new(scope, &json).unwrap().into());
        },
    )
    .unwrap();
    global.set(
        scope,
        v8::String::new(scope, "__eventSourcePoll").unwrap().into(),
        poll_fn.into(),
    );

    let close_fn = v8::Function::new(
        scope,
        move |scope: &mut v8::PinScope,
              args: v8::FunctionCallbackArguments,
              mut _rv: v8::ReturnValue| {
            let id = args.get(0).uint32_value(scope).unwrap_or(0);
            let map_arc = map();
            let mut m = map_arc.lock().unwrap();
            if let Some(entry) = m.remove(&id) {
                *entry.cancel.lock().unwrap() = true;
            }
        },
    )
    .unwrap();
    global.set(
        scope,
        v8::String::new(scope, "__eventSourceClose").unwrap().into(),
        close_fn.into(),
    );
}
