# Event Enrichment

## What It Demonstrates

A realistic event processing pipeline: structured log events are enriched with geo-IP data (simulated), classified by severity, and routed to an alerting sink for high-severity events. Demonstrates multi-stage enrichment, a common pattern in observability and security systems.

## Pipeline Topology

```
event-source → geo-enricher → severity-classifier → alert-sink
```

## Key Components

The **geo-enricher** processor reads an event's IP address field, looks it up in a simulated geo-IP table (loaded at `init()`), and adds `country` and `city` fields to the JSON.

The **severity-classifier** processor examines the event's `level` field and a set of keyword rules to assign a severity score. Events with severity above a threshold are annotated with `"alert": true`.

The **alert-sink** prints only events where `"alert": true`, ignoring routine events.

### Source Component: `event-source`

**`components/event-source/src/lib.rs`**

```rust
//! Structured log event source.
//!
//! Produces a sequence of JSON log events with varying severity levels
//! and IP addresses for downstream enrichment and classification.

wit_bindgen::generate!({
    world: "data-source",
    path: "../../wit/torvyn-streaming",
});

use exports::torvyn::streaming::lifecycle::Guest as LifecycleGuest;
use exports::torvyn::streaming::source::Guest as SourceGuest;
use torvyn::streaming::buffer_allocator;
use torvyn::streaming::types::*;

const EVENTS: &[&str] = &[
    r#"{"ip":"192.0.2.10","level":"info","message":"user login successful"}"#,
    r#"{"ip":"198.51.100.42","level":"warn","message":"high memory usage detected"}"#,
    r#"{"ip":"203.0.113.5","level":"error","message":"disk full on /data"}"#,
    r#"{"ip":"192.0.2.10","level":"info","message":"config reload completed"}"#,
    r#"{"ip":"198.51.100.1","level":"critical","message":"auth service down"}"#,
];

struct EventSource {
    index: usize,
}

static mut STATE: Option<EventSource> = None;
fn state() -> &'static mut EventSource {
    unsafe { STATE.as_mut().expect("not initialized") }
}

impl LifecycleGuest for EventSource {
    fn init(_config: String) -> Result<(), ProcessError> {
        unsafe { STATE = Some(EventSource { index: 0 }); }
        Ok(())
    }
    fn teardown() { unsafe { STATE = None; } }
}

impl SourceGuest for EventSource {
    fn pull() -> Result<Option<OutputElement>, ProcessError> {
        let s = state();
        if s.index >= EVENTS.len() {
            return Ok(None);
        }
        let payload = EVENTS[s.index].as_bytes();
        let buf = buffer_allocator::allocate(payload.len() as u64)
            .map_err(|e| ProcessError::Internal(format!("{e:?}")))?;
        buf.append(payload)
            .map_err(|e| ProcessError::Internal(format!("{e:?}")))?;
        buf.set_content_type("application/json");
        let frozen = buf.freeze();
        s.index += 1;
        Ok(Some(OutputElement {
            meta: ElementMeta {
                sequence: (s.index - 1) as u64,
                timestamp_ns: 0,
                content_type: "application/json".to_string(),
            },
            payload: frozen,
        }))
    }
    fn notify_backpressure(_signal: BackpressureSignal) {}
}

export!(EventSource);
```

### Processor Component: `geo-enricher`

**`components/geo-enricher/src/lib.rs`**

```rust
//! Geo-IP enrichment processor.
//!
//! Reads the "ip" field from each JSON event and adds "country" and "city"
//! fields based on a simulated lookup table. In a production deployment,
//! this would use a real geo-IP database loaded at init().
//!
//! Demonstrates: stateful processor with lookup table loaded during
//! lifecycle initialization.

wit_bindgen::generate!({
    world: "managed-transform",
    path: "../../wit/torvyn-streaming",
});

use exports::torvyn::streaming::lifecycle::Guest as LifecycleGuest;
use exports::torvyn::streaming::processor::Guest as ProcessorGuest;
use torvyn::streaming::buffer_allocator;
use torvyn::streaming::types::*;

struct GeoEnricher {
    /// Simulated geo-IP database: IP → (country, city).
    db: Vec<(&'static str, &'static str, &'static str)>,
}

static mut STATE: Option<GeoEnricher> = None;
fn state() -> &'static mut GeoEnricher {
    unsafe { STATE.as_mut().expect("not initialized") }
}

impl LifecycleGuest for GeoEnricher {
    fn init(_config: String) -> Result<(), ProcessError> {
        // In production, load a real geo-IP database from config path.
        // This simulated table maps example IPs to locations.
        let db = vec![
            ("192.0.2.10",    "DE", "Berlin"),
            ("198.51.100.42", "JP", "Tokyo"),
            ("203.0.113.5",   "AU", "Sydney"),
            ("198.51.100.1",  "US", "Chicago"),
        ];
        unsafe { STATE = Some(GeoEnricher { db }); }
        Ok(())
    }
    fn teardown() { unsafe { STATE = None; } }
}

impl ProcessorGuest for GeoEnricher {
    fn process(input: StreamElement) -> Result<ProcessResult, ProcessError> {
        let s = state();
        let bytes = input.payload.read_all();
        let text = String::from_utf8(bytes)
            .map_err(|e| ProcessError::InvalidInput(format!("{e}")))?;

        // Extract IP address using simple string search.
        // Production code should use a proper JSON parser.
        let ip = extract_json_string(&text, "ip").unwrap_or_default();

        // Look up geo data.
        let (country, city) = s.db.iter()
            .find(|(db_ip, _, _)| *db_ip == ip.as_str())
            .map(|(_, country, city)| (*country, *city))
            .unwrap_or(("unknown", "unknown"));

        // Append geo fields to the JSON.
        let enriched = text.trim_end_matches('}').to_string()
            + &format!(",\"country\":\"{}\",\"city\":\"{}\"}}", country, city);

        let out_bytes = enriched.as_bytes();
        let buf = buffer_allocator::allocate(out_bytes.len() as u64)
            .map_err(|e| ProcessError::Internal(format!("{e:?}")))?;
        buf.append(out_bytes)
            .map_err(|e| ProcessError::Internal(format!("{e:?}")))?;
        buf.set_content_type("application/json");
        let frozen = buf.freeze();

        Ok(ProcessResult::Emit(OutputElement {
            meta: ElementMeta {
                sequence: input.meta.sequence,
                timestamp_ns: 0,
                content_type: "application/json".to_string(),
            },
            payload: frozen,
        }))
    }
}

/// Simple JSON string field extractor (no serde dependency).
/// Finds `"key":"value"` and returns value.
fn extract_json_string(json: &str, key: &str) -> Option<String> {
    let pattern = format!("\"{}\":\"", key);
    let start = json.find(&pattern)? + pattern.len();
    let rest = &json[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

export!(GeoEnricher);
```

### Processor Component: `severity-classifier`

**`components/severity-classifier/src/lib.rs`**

```rust
//! Severity classifier processor.
//!
//! Examines each event's "level" field, assigns a numeric severity
//! score, and adds "severity" and "alert" fields. Events with
//! severity above the configured threshold are marked with
//! "alert": true.

wit_bindgen::generate!({
    world: "managed-transform",
    path: "../../wit/torvyn-streaming",
});

use exports::torvyn::streaming::lifecycle::Guest as LifecycleGuest;
use exports::torvyn::streaming::processor::Guest as ProcessorGuest;
use torvyn::streaming::buffer_allocator;
use torvyn::streaming::types::*;

struct SeverityClassifier {
    alert_threshold: u32,
}

static mut STATE: Option<SeverityClassifier> = None;
fn state() -> &'static mut SeverityClassifier {
    unsafe { STATE.as_mut().expect("not initialized") }
}

impl LifecycleGuest for SeverityClassifier {
    fn init(config: String) -> Result<(), ProcessError> {
        // Parse threshold from JSON config: {"alert_threshold": 7}
        let threshold = config
            .trim()
            .strip_prefix("{\"alert_threshold\":")
            .and_then(|s| s.strip_suffix('}'))
            .and_then(|s| s.trim().parse::<u32>().ok())
            .unwrap_or(7);
        unsafe { STATE = Some(SeverityClassifier { alert_threshold: threshold }); }
        Ok(())
    }
    fn teardown() { unsafe { STATE = None; } }
}

impl ProcessorGuest for SeverityClassifier {
    fn process(input: StreamElement) -> Result<ProcessResult, ProcessError> {
        let s = state();
        let bytes = input.payload.read_all();
        let text = String::from_utf8(bytes)
            .map_err(|e| ProcessError::InvalidInput(format!("{e}")))?;

        // Extract level field.
        let level = extract_json_string(&text, "level").unwrap_or_default();

        // Map level to numeric severity.
        let severity: u32 = match level.as_str() {
            "debug"    => 1,
            "info"     => 3,
            "warn"     => 5,
            "error"    => 9,
            "critical" => 10,
            _          => 0,
        };

        let alert = severity >= s.alert_threshold;

        // Append severity and alert fields.
        let classified = text.trim_end_matches('}').to_string()
            + &format!(",\"severity\":{},\"alert\":{}}}", severity, alert);

        let out_bytes = classified.as_bytes();
        let buf = buffer_allocator::allocate(out_bytes.len() as u64)
            .map_err(|e| ProcessError::Internal(format!("{e:?}")))?;
        buf.append(out_bytes)
            .map_err(|e| ProcessError::Internal(format!("{e:?}")))?;
        buf.set_content_type("application/json");
        let frozen = buf.freeze();

        Ok(ProcessResult::Emit(OutputElement {
            meta: ElementMeta {
                sequence: input.meta.sequence,
                timestamp_ns: 0,
                content_type: "application/json".to_string(),
            },
            payload: frozen,
        }))
    }
}

fn extract_json_string(json: &str, key: &str) -> Option<String> {
    let pattern = format!("\"{}\":\"", key);
    let start = json.find(&pattern)? + pattern.len();
    let rest = &json[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

export!(SeverityClassifier);
```

### Sink Component: `alert-sink`

**`components/alert-sink/src/lib.rs`**

```rust
//! Alert sink.
//!
//! Prints events where "alert":true. Suppresses routine events
//! and reports a count at the end.

wit_bindgen::generate!({
    world: "data-sink",
    path: "../../wit/torvyn-streaming",
});

use exports::torvyn::streaming::lifecycle::Guest as LifecycleGuest;
use exports::torvyn::streaming::sink::Guest as SinkGuest;
use torvyn::streaming::types::*;

struct AlertSink {
    alert_count: u64,
    suppressed_count: u64,
}

static mut STATE: Option<AlertSink> = None;
fn state() -> &'static mut AlertSink {
    unsafe { STATE.as_mut().expect("not initialized") }
}

impl LifecycleGuest for AlertSink {
    fn init(_config: String) -> Result<(), ProcessError> {
        unsafe {
            STATE = Some(AlertSink {
                alert_count: 0,
                suppressed_count: 0,
            });
        }
        Ok(())
    }
    fn teardown() { unsafe { STATE = None; } }
}

impl SinkGuest for AlertSink {
    fn push(element: StreamElement) -> Result<BackpressureSignal, ProcessError> {
        let s = state();
        let bytes = element.payload.read_all();
        let text = String::from_utf8(bytes)
            .map_err(|e| ProcessError::InvalidInput(format!("{e}")))?;

        // Check if this event is an alert.
        if text.contains("\"alert\":true") {
            println!(
                "[alert-sink] ALERT seq={}: {}",
                element.meta.sequence, text
            );
            s.alert_count += 1;
        } else {
            s.suppressed_count += 1;
        }

        Ok(BackpressureSignal::Ready)
    }

    fn complete() -> Result<(), ProcessError> {
        let s = state();
        println!(
            "[alert-sink] Non-alert events: {} (suppressed)",
            s.suppressed_count
        );
        println!("[alert-sink] Stream complete.");
        Ok(())
    }
}

export!(AlertSink);
```

## Pipeline Configuration

**`Torvyn.toml`**

```toml
[torvyn]
name = "event-enrichment"
version = "0.1.0"
contract_version = "0.1.0"
description = "Multi-stage event enrichment pipeline"

[[component]]
name = "event-source"
path = "components/event-source"

[[component]]
name = "geo-enricher"
path = "components/geo-enricher"

[[component]]
name = "severity-classifier"
path = "components/severity-classifier"

[[component]]
name = "alert-sink"
path = "components/alert-sink"

[flow.main]
description = "Events → Geo-IP → Severity → Alert"

[flow.main.nodes.source]
component = "event-source"
interface = "torvyn:streaming/source"

[flow.main.nodes.geo]
component = "geo-enricher"
interface = "torvyn:streaming/processor"
config = '{"db": "simulated"}'

[flow.main.nodes.severity]
component = "severity-classifier"
interface = "torvyn:streaming/processor"
config = '{"alert_threshold": 7}'

[flow.main.nodes.sink]
component = "alert-sink"
interface = "torvyn:streaming/sink"

[[flow.main.edges]]
from = { node = "source", port = "output" }
to = { node = "geo", port = "input" }

[[flow.main.edges]]
from = { node = "geo", port = "output" }
to = { node = "severity", port = "input" }

[[flow.main.edges]]
from = { node = "severity", port = "output" }
to = { node = "sink", port = "input" }
```

## Expected Output

```
$ torvyn run flow.main
[torvyn] Running flow 'main'

[alert-sink] ALERT seq=2: {"ip":"203.0.113.5","level":"error","message":"disk full on /data","country":"AU","city":"Sydney","severity":9,"alert":true}
[alert-sink] ALERT seq=4: {"ip":"198.51.100.1","level":"critical","message":"auth service down","country":"US","city":"Chicago","severity":10,"alert":true}
[alert-sink] Non-alert events: 3 (suppressed)
[alert-sink] Stream complete.

[torvyn] Flow 'main' completed. 5 events processed, 2 alerts raised.
```

## Performance Characteristics (Design Targets)

| Metric | Target |
|--------|--------|
| Enrichment latency per event | < 100 us (simulated lookup) |
| Copies per event | 3 (source write, geo-enricher transform, classifier transform) |
| Memory for geo-IP table | Proportional to table size (loaded at init) |

## Learn More

- [Use Case Guide: Event Processing](docs/use-cases/event-processing.md)
- [Architecture Guide: Multi-Stage Pipelines](docs/architecture.md#multi-stage)
