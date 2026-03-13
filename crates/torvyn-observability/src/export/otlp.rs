//! Minimal OTLP HTTP/JSON trace exporter.
//!
//! Per MR-11: does not depend on the `opentelemetry` Rust SDK.
//! Implements the minimum subset of the OTLP trace protocol needed
//! to export span data to an OTel Collector or compatible backend.
//!
//! LLI DEVIATION: The OpenTelemetry Rust SDK remains unstable as of
//! early 2025. Per MR-11 decision, we implement a minimal OTLP
//! HTTP/JSON exporter instead of depending on the SDK.

use crate::tracer::CompactSpanRecord;
use serde::{Deserialize, Serialize};
use torvyn_types::TraceId;

/// An OTLP-compatible span for JSON export.
///
/// Subset of the full OpenTelemetry Span proto, containing only
/// the fields Torvyn populates.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OtlpSpan {
    /// W3C trace ID (32 hex chars).
    pub trace_id: String,
    /// Span ID (16 hex chars).
    pub span_id: String,
    /// Parent span ID (16 hex chars).
    pub parent_span_id: String,
    /// Human-readable span name.
    pub name: String,
    /// Start time in nanoseconds since Unix epoch.
    pub start_time_unix_nano: u64,
    /// End time in nanoseconds since Unix epoch.
    pub end_time_unix_nano: u64,
    /// Span status.
    pub status: OtlpStatus,
    /// Span attributes.
    pub attributes: Vec<OtlpAttribute>,
}

/// OTLP span status.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OtlpStatus {
    /// Status code: 0=Unset, 1=Ok, 2=Error.
    pub code: u32,
    /// Optional status message.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// OTLP key-value attribute.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OtlpAttribute {
    /// Attribute key.
    pub key: String,
    /// Attribute value.
    pub value: OtlpValue,
}

/// OTLP attribute value (string or integer).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OtlpValue {
    /// String value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub string_value: Option<String>,
    /// Integer value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub int_value: Option<i64>,
}

/// Convert a batch of compact span records to OTLP-compatible JSON.
///
/// # COLD PATH — called during export flush.
pub fn spans_to_otlp(trace_id: TraceId, spans: &[CompactSpanRecord]) -> Vec<OtlpSpan> {
    let trace_id_hex = format!("{trace_id}");

    spans
        .iter()
        .map(|s| {
            let status_code = match s.status_code {
                0 => 1, // Ok
                _ => 2, // Error
            };

            OtlpSpan {
                trace_id: trace_id_hex.clone(),
                span_id: format!("{}", s.span_id),
                parent_span_id: format!("{}", s.parent_span_id),
                name: format!("component:{}", s.component_id),
                start_time_unix_nano: s.start_ns,
                end_time_unix_nano: s.end_ns,
                status: OtlpStatus {
                    code: status_code,
                    message: None,
                },
                attributes: vec![
                    OtlpAttribute {
                        key: "torvyn.component.id".into(),
                        value: OtlpValue {
                            int_value: Some(s.component_id.as_u64() as i64),
                            string_value: None,
                        },
                    },
                    OtlpAttribute {
                        key: "torvyn.element.sequence".into(),
                        value: OtlpValue {
                            int_value: Some(s.element_sequence as i64),
                            string_value: None,
                        },
                    },
                ],
            }
        })
        .collect()
}

/// OTLP export request body (minimal subset).
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OtlpExportRequest {
    /// Resource spans envelope.
    pub resource_spans: Vec<OtlpResourceSpans>,
}

/// OTLP resource spans container.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OtlpResourceSpans {
    /// Scope spans within this resource.
    pub scope_spans: Vec<OtlpScopeSpans>,
}

/// OTLP scope spans container.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OtlpScopeSpans {
    /// Spans within this scope.
    pub spans: Vec<OtlpSpan>,
}

/// Build an OTLP export request from a batch of spans.
pub fn build_export_request(spans: Vec<OtlpSpan>) -> OtlpExportRequest {
    OtlpExportRequest {
        resource_spans: vec![OtlpResourceSpans {
            scope_spans: vec![OtlpScopeSpans { spans }],
        }],
    }
}

// IMPLEMENTATION NOTE: The actual HTTP POST to the OTLP endpoint
// requires an HTTP client (e.g., reqwest or hyper). Since the
// network allowlist may not include arbitrary OTLP endpoints,
// the HTTP send is gated behind the `otlp-export` feature flag.
//
// EXTERNAL API ASSUMPTION: OTLP HTTP/JSON endpoint accepts POST to
// /v1/traces with Content-Type: application/json. Verify against
// OpenTelemetry specification v1.x.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tracer::CompactSpanRecord;
    use torvyn_types::{ComponentId, SpanId};

    #[test]
    fn test_spans_to_otlp() {
        let trace_id = TraceId::new([0xab; 16]);
        let spans = vec![CompactSpanRecord {
            span_id: SpanId::new([1; 8]),
            parent_span_id: SpanId::new([0; 8]),
            component_id: ComponentId::new(42),
            start_ns: 1000,
            end_ns: 2000,
            status_code: 0,
            element_sequence: 1,
        }];

        let otlp = spans_to_otlp(trace_id, &spans);
        assert_eq!(otlp.len(), 1);
        assert_eq!(otlp[0].start_time_unix_nano, 1000);
        assert_eq!(otlp[0].end_time_unix_nano, 2000);
        assert_eq!(otlp[0].status.code, 1); // Ok
    }

    #[test]
    fn test_spans_to_otlp_error_status() {
        let trace_id = TraceId::new([0xab; 16]);
        let spans = vec![CompactSpanRecord {
            span_id: SpanId::new([1; 8]),
            parent_span_id: SpanId::new([0; 8]),
            component_id: ComponentId::new(1),
            start_ns: 0,
            end_ns: 100,
            status_code: 1, // error
            element_sequence: 0,
        }];

        let otlp = spans_to_otlp(trace_id, &spans);
        assert_eq!(otlp[0].status.code, 2); // Error
    }

    #[test]
    fn test_build_export_request() {
        let otlp_spans = vec![OtlpSpan {
            trace_id: "abc".into(),
            span_id: "def".into(),
            parent_span_id: "000".into(),
            name: "test".into(),
            start_time_unix_nano: 0,
            end_time_unix_nano: 100,
            status: OtlpStatus {
                code: 1,
                message: None,
            },
            attributes: vec![],
        }];

        let req = build_export_request(otlp_spans);
        assert_eq!(req.resource_spans.len(), 1);
        assert_eq!(req.resource_spans[0].scope_spans[0].spans.len(), 1);

        // Verify serialization doesn't panic.
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("test"));
    }
}
