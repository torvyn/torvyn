module torvyn.dev/test-components/go-echo-source

// Must not exceed the Go toolchain TinyGo ships with (TinyGo 0.41 uses
// go1.24.x); a higher directive makes `tinygo build` reject the module.
// `go.bytecodealliance.org/cm` requires go 1.23.0, so 1.24 satisfies both.
go 1.24

require go.bytecodealliance.org/cm v0.3.0
