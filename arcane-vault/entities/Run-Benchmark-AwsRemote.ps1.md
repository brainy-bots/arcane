---
type: entity
tags: [benchmark, aws-infrastructure, powershell, scripting, orchestration, arcane]
---

# Run-Benchmark-AwsRemote.ps1

## What It Is
`Run-Benchmark-AwsRemote.ps1` is the AWS-aware orchestration layer in Arcane's benchmark scripting infrastructure. It sits between the provisioning script and the core run harness: it reads infrastructure state written by `Setup-AwsBenchmark.ps1` and delegates the actual workload execution to `Run-Benchmark.ps1`. Its job is to bridge provisioned cloud state to the engine-agnostic benchmark runner.

## Origin & Evolution
The script emerged from a critical insight during the 2026-03-30 session: provisioning, running, and cleanup had been incorrectly conflated inside a single `Run-Benchmark.ps1`. The session established a clean four-way separation of concerns — provision, run-remote, run-local, and teardown — each as a distinct script with a single responsibility. `Run-Benchmark-AwsRemote.ps1` was defined as the glue layer that knows about AWS state but delegates benchmark logic. As part of the same session, the broader directory layout was restructured: `scripts/cloud/` became `infra/aws/`, `Common/` became `lib/`, and `environments/` became `topologies/`, with all import paths updated accordingly.

## Technical Details
The script reads a state JSON file produced by `Setup-AwsBenchmark.ps1` that captures provisioned infrastructure details (host addresses, topology, etc.). It extracts the relevant parameters and forwards them to `Run-Benchmark.ps1`, which assumes infrastructure already exists and focuses solely on workload execution. Multi-host Arcane parameters that had leaked into the run harness were identified and removed during refactoring, keeping cloud-specific concerns isolated in this script rather than polluting the core harness.

## Key Design Decisions
- **State JSON handoff** — provisioning writes a JSON file; this script reads it, decoupling the provision and run phases so they can run in separate CI jobs or be retried independently
- **Delegation to Run-Benchmark.ps1** — the remote script does not re-implement benchmark logic; it translates AWS context into harness arguments, keeping the core harness testable without cloud dependencies
- **Multi-host parameters removed from harness** — cloud-topology-specific parameters belong here, not in `Run-Benchmark.ps1`, preventing the core runner from accumulating AWS-specific concerns
- **Part of the four-script model** — the clean split into Setup / RunRemote / Run / Cleanup emerged explicitly to prevent the anti-pattern of a single monolithic script doing everything

## Relationships
- [[Run-Benchmark.ps1]] — core harness this script delegates to
- [[Setup-AwsBenchmark.ps1]] — produces the state JSON this script consumes
- [[Cleanup-AwsBenchmark.ps1]] — the teardown counterpart in the four-script model
- [[infra/aws/]] — directory home after the `scripts/cloud/` restructure
- [[infra/aws/lib/]] — shared library code (formerly `Common/`)
- [[infra/aws/topologies/]] — topology definitions (formerly `environments/`)

## Conversations That Shaped This
- [[Benchmark improvement suggestions]]