# Security Notes

CFMD contains authenticated deployment, replication evidence and durable-store trust boundaries. Security-sensitive changes should preserve fail-closed behavior and use established cryptographic implementations rather than project-local cryptography.

The vendored dependency closure is intentional for reproducible/offline builds. Dependency changes should update `Cargo.lock`, vendor sources and the relevant audit/evidence record together.

Durability certification is not a universal hardware claim. Only exact certified platform profiles are admitted as certified operation.
