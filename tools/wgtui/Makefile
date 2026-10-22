CARGO ?= cargo

.PHONY: check test demo release package package-universal
check:
	$(CARGO) fmt --check
	$(CARGO) check --locked

test:
	$(CARGO) test --locked

demo:
	$(CARGO) run --locked -- --demo

release:
	$(CARGO) build --release --locked

# Uruchom osobno dla każdego celu po jego kompilacji.
# make package TARGET=x86_64-unknown-linux-musl ARCH=amd64
package:
	python3 scripts/package.py --target $(TARGET) --arch $(ARCH)

# Oba cele muszą być wcześniej skompilowane.
package-universal:
	python3 scripts/package.py --universal
