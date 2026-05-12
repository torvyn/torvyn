// Polyglot proof: Go (TinyGo) `data-source` component for the Torvyn
// streaming runtime. Behaves identically to the Rust `echo-source`
// fixture — produces N numbered elements through the host's
// `buffer-allocator` import — so the host's `DefaultResourceManager`
// records the same per-element copy events regardless of which guest
// language emitted them.
//
// Configuration string passed to `lifecycle.init`:
//
//	{"count":N}
//
// Empty config falls back to 1000 elements.
package main

import (
	"strconv"
	"strings"

	"go.bytecodealliance.org/cm"

	bufferallocator "torvyn.dev/test-components/go-echo-source/internal/torvyn/streaming/buffer-allocator"
	"torvyn.dev/test-components/go-echo-source/internal/torvyn/streaming/lifecycle"
	"torvyn.dev/test-components/go-echo-source/internal/torvyn/streaming/source"
	"torvyn.dev/test-components/go-echo-source/internal/torvyn/streaming/types"
)

// state is the source's monotonic sequence + remaining counter. The
// component runs single-threaded inside a Wasm instance — no
// synchronisation is needed.
var state struct {
	remaining uint64
	sequence  uint64
}

// pullResult and initResult alias the bindgen-generated Result types so
// the call sites below stay readable.
type pullResult = cm.Result[source.OptionOutputElementShape, cm.Option[types.OutputElement], types.ProcessError]
type initResult = cm.Result[types.ProcessError, struct{}, types.ProcessError]

// init wires the bindgen-generated `Exports` function variables to the
// Go handlers. TinyGo runs `init` before the host can call any export.
func init() {
	lifecycle.Exports.Init = handleInit
	lifecycle.Exports.Teardown = handleTeardown
	source.Exports.Pull = handlePull
	source.Exports.NotifyBackpressure = handleNotifyBackpressure
}

// handleInit parses the `{"count":N}` configuration string and seeds
// the source's state. Missing or malformed input falls back to 1000 so
// the component remains useful when a developer forgets the config.
func handleInit(config string) initResult {
	count := uint64(1000)
	if config != "" {
		if parsed, ok := parseCountJSON(config); ok {
			count = parsed
		}
	}
	state.remaining = count
	state.sequence = 0
	return cm.OK[initResult, types.ProcessError, struct{}, types.ProcessError](struct{}{})
}

func handleTeardown() {}

// handlePull emits a single element per call, encoded as the
// little-endian 8-byte sequence number, until the configured count is
// exhausted. Each call drives the source's manager-side state machine
// exactly once: allocate → write → freeze → return-owned.
func handlePull() pullResult {
	if state.remaining == 0 {
		return cm.OK[pullResult, source.OptionOutputElementShape, cm.Option[types.OutputElement], types.ProcessError](
			cm.None[types.OutputElement](),
		)
	}
	seq := state.sequence
	state.sequence++
	state.remaining--

	allocRes := bufferallocator.Allocate(8)
	if allocRes.IsErr() {
		return cm.Err[pullResult, source.OptionOutputElementShape, cm.Option[types.OutputElement], types.ProcessError](
			types.ProcessErrorInternal("buffer-allocator.allocate failed"),
		)
	}
	mb := *allocRes.OK()

	payload := encodeSequenceLE(seq)
	writeRes := mb.Write(0, cm.ToList(payload))
	if writeRes.IsErr() {
		return cm.Err[pullResult, source.OptionOutputElementShape, cm.Option[types.OutputElement], types.ProcessError](
			types.ProcessErrorInternal("mutable-buffer.write failed"),
		)
	}

	buf := mb.Freeze()

	return cm.OK[pullResult, source.OptionOutputElementShape, cm.Option[types.OutputElement], types.ProcessError](
		cm.Some(types.OutputElement{
			Meta: types.ElementMeta{
				Sequence:    seq,
				TimestampNs: 0,
				ContentType: "application/octet-stream",
			},
			Payload: buf,
		}),
	)
}

func handleNotifyBackpressure(_ types.BackpressureSignal) {}

// encodeSequenceLE returns the little-endian 8-byte encoding of seq.
// Mirrors the Rust source's `seq.to_le_bytes()` exactly so the polyglot
// pipeline produces byte-for-byte identical wire payloads to the
// all-Rust pipeline.
func encodeSequenceLE(seq uint64) []byte {
	return []byte{
		byte(seq),
		byte(seq >> 8),
		byte(seq >> 16),
		byte(seq >> 24),
		byte(seq >> 32),
		byte(seq >> 40),
		byte(seq >> 48),
		byte(seq >> 56),
	}
}

// parseCountJSON is a minimal handcrafted parser for the
// `{"count":N}` shape — TinyGo's standard library does not include the
// full `encoding/json` package, and pulling a third-party JSON
// dependency just for this test fixture would inflate the binary
// without benefit. The parser is intentionally strict: anything that
// doesn't match the exact shape yields `ok == false` and the caller
// uses the default count.
func parseCountJSON(config string) (uint64, bool) {
	trimmed := strings.TrimSpace(config)
	trimmed = strings.TrimPrefix(trimmed, "{\"count\":")
	if !strings.HasSuffix(trimmed, "}") {
		return 0, false
	}
	trimmed = strings.TrimSuffix(trimmed, "}")
	n, err := strconv.ParseUint(strings.TrimSpace(trimmed), 10, 64)
	if err != nil {
		return 0, false
	}
	return n, true
}

// main is required by TinyGo's `wasip2` target even though the
// component model entry points are driven by the exported functions.
// Leaving the function body empty is conventional for guest components.
func main() {}
