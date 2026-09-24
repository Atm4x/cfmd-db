.PHONY: verify check test formal stats

verify:
	./scripts/verify-repository.sh

check:
	./scripts/ci-rust.sh

test:
	CARGO_NET_OFFLINE=true cargo test --workspace --all-targets --locked --offline

formal:
	./scripts/ci-formal.sh

stats:
	./scripts/stats.sh
