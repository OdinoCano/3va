use rquickjs::{Ctx, Function, Result};
use serde::Serialize;
use std::collections::HashMap;
use std::io::BufRead;
use std::sync::{Arc, Mutex};

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

pub fn inject_event_source(ctx: &Ctx) -> Result<()> {
    let map: EsMap = Arc::new(Mutex::new(HashMap::new()));
    let counter = Arc::new(Mutex::new(0u32));

    // __eventSourceOpen(url) → id
    let map2 = map.clone();
    let counter2 = counter.clone();
    ctx.globals().set(
        "__eventSourceOpen",
        Function::new(ctx.clone(), move |url: String| -> rquickjs::Result<u32> {
            let mut id_lock = counter2.lock().unwrap();
            *id_lock += 1;
            let id = *id_lock;
            drop(id_lock);

            let queue: EventQueue = Arc::new(Mutex::new(Vec::new()));
            let cancel = Arc::new(Mutex::new(false));

            let q2 = queue.clone();
            let c2 = cancel.clone();
            let url2 = url.clone();

            // Spawn a blocking thread that streams the SSE response
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
                        // Dispatch event
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
                    // ignore retry: and comments (:)
                }
            });

            map2.lock().unwrap().insert(
                id,
                EsEntry {
                    events: queue,
                    cancel,
                },
            );
            Ok(id)
        })?,
    )?;

    // __eventSourcePoll(id) → JSON string of pending events (or null)
    let map2 = map.clone();
    ctx.globals().set(
        "__eventSourcePoll",
        Function::new(
            ctx.clone(),
            move |id: u32| -> rquickjs::Result<Option<String>> {
                let m = map2.lock().unwrap();
                let entry = match m.get(&id) {
                    Some(e) => e,
                    None => return Ok(None),
                };
                let mut q = entry.events.lock().unwrap();
                if q.is_empty() {
                    return Ok(Some("[]".to_string()));
                }
                let drained: Vec<SseEvent> = q.drain(..).collect();
                drop(q);
                Ok(Some(
                    serde_json::to_string(&drained).unwrap_or_else(|_| "[]".to_string()),
                ))
            },
        )?,
    )?;

    // __eventSourceClose(id)
    let map2 = map.clone();
    ctx.globals().set(
        "__eventSourceClose",
        Function::new(ctx.clone(), move |id: u32| {
            let mut m = map2.lock().unwrap();
            if let Some(entry) = m.remove(&id) {
                *entry.cancel.lock().unwrap() = true;
            }
        })?,
    )?;

    Ok(())
}
