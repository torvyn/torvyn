# Security Policy

The Torvyn project takes the security of its software seriously. If you have discovered a security issue, we appreciate your help in disclosing it to us responsibly.

---

## Reporting a Security Issue

**Please do not report security issues through public GitHub issues, discussions, or pull requests.**

Instead, please report security issues through [GitHub Security Advisories](https://github.com/torvyn/torvyn/security/advisories/new).

### What to Include in Your Report

To help us triage and respond efficiently, please include:

- A description of the issue and its potential impact.
- Step-by-step instructions to reproduce the issue, or a proof-of-concept if available.
- The Torvyn version(s) affected.
- Your environment (OS, Rust toolchain version, Wasmtime version if relevant).
- Any potential mitigations you have identified.

You will receive an acknowledgment within **48 hours** confirming that we received your report.

---

## Response Timeline

| Stage | Target Timeline |
|-------|----------------|
| Acknowledgment of report | Within 48 hours |
| Initial triage and severity assessment | Within 5 business days |
| Fix development begins | Within 7 business days of triage |
| Fix available (patch release or mitigation) | Varies by severity (see below) |
| Public disclosure | After fix is available, coordinated with reporter |

These timelines are commitments, not aspirations. If we cannot meet a timeline, we will notify the reporter with an explanation and revised estimate.

---

## Severity Classification

Torvyn classifies security issues using four severity levels:

### Critical

Issues that allow running code on the host without authorization, escaping the WebAssembly sandbox, or compromising the integrity of the host runtime from guest component code.

**Target resolution:** Patch release within 7 days of confirmed triage.

### High

Issues that allow a component to access resources beyond its granted capabilities, bypass capability enforcement, read another component's memory, or cause denial-of-service against the host runtime.

**Target resolution:** Patch release within 14 days of confirmed triage.

### Medium

Issues that allow information leakage between components (timing side-channels, observable resource contention patterns), bypass of non-critical policy enforcement, or degradation of observability integrity (trace spoofing, metric manipulation).

**Target resolution:** Fix included in the next scheduled release (within 30 days).

### Low

Issues with minimal direct impact, such as theoretical issues with no practical path to exploitation, documentation errors in security guidance, or minor deviations from stated security properties with no user-visible impact.

**Target resolution:** Fix included in the next scheduled release.

---

## Disclosure Process

Torvyn follows a **coordinated disclosure** model:

1. **Embargo period:** Once a report is confirmed, the details remain private while a fix is developed. The standard embargo period is 90 days from the initial report. If a fix is ready sooner, disclosure happens sooner.
2. **Reporter coordination:** Before public disclosure, we coordinate with the reporter on timing. The reporter receives advance notice of the fix and the planned disclosure date.
3. **Pre-notification:** For Critical and High severity issues, we may pre-notify known significant downstream users (e.g., organizations that have registered for security notifications) up to 7 days before public disclosure, under embargo.
4. **Public disclosure:** A security advisory is published on GitHub (via GitHub Security Advisories), including the CVE identifier (if assigned), affected versions, a description of the issue, the fix, and upgrade instructions.
5. **Credit:** The reporter is credited in the advisory by default. Reporters may request to remain anonymous.

---

## Security Update Notifications

To receive notifications about security updates:

- **Watch the repository** on GitHub with "Security Advisories" notifications enabled.
- **Check the GitHub Security Advisories page** for the Torvyn repository.

---

## Supported Versions

| Version | Security Patches |
|---------|-----------------|
| Latest `0.x` release | Yes — active development |
| Previous `0.x` release | Best-effort, Critical and High only |
| Older releases | Not supported |

Once Torvyn reaches 1.0, the support policy will be formalized to cover the current major release and the previous major release for at least 12 months after a new major release.

---

## Security Architecture

Torvyn's security model is designed with defense in depth:

- **WebAssembly sandboxing:** Components run within Wasm linear memory boundaries. Memory isolation between components is enforced by the Wasm engine (Wasmtime).
- **Capability-based isolation:** Components receive only the permissions explicitly granted to them. Capabilities are declared in WIT contracts and enforced at link time and runtime.
- **Host-managed resources:** All buffer memory is owned and tracked by the host runtime. Components cannot access buffers they do not hold a valid handle for.
- **Fuel-based CPU budgeting:** Components are allocated a CPU budget (fuel). Budget exhaustion causes cooperative preemption, not host-level starvation.
- **Memory limits:** Per-component memory limits prevent any single component from exhausting host memory.

For details on the security model, see the [Architecture Guide](documents/ARCHITECTURE.md) and the security design document in `docs/design/`.

---

## Scope

This security policy applies to:

- The Torvyn host runtime (`torvyn-host` and all workspace crates).
- The Torvyn CLI (`torvyn-cli`).
- Official WIT contract packages (`torvyn:streaming`, `torvyn:capabilities`, etc.).
- Official example components and templates.
- The project's CI/CD infrastructure and release signing.

Third-party components published to registries are outside the scope of this policy. However, if a weakness in a third-party component reveals a flaw in Torvyn's isolation model, that isolation issue is in scope.
