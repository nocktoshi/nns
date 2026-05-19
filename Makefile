PREFIX ?= $(HOME)/.local
BINDIR := $(PREFIX)/bin
LIBDIR := $(PREFIX)/lib/nns
WRAPPER := $(BINDIR)/nns
BIN := $(LIBDIR)/nns
KERNEL := $(LIBDIR)/nns.jam
SHELL_RC ?= $(HOME)/.zshrc

KERNEL_JAM := nns.jam
KERNEL_CACHE := .cache
KERNEL_BUILT_HASH := $(KERNEL_CACHE)/kernel-built.hash
NOCKUP_INSTALLED := hoon/packages/.installed
NOCKCHAIN_GIT := https://github.com/nocktoshi/nockchain.git
NOCKCHAIN_BRANCH := dev

VESL_LIB_NAMES := vesl-graft vesl-merkle vesl-prover vesl-stark-verifier vesl-verifier

.PHONY: install install-rust uninstall install-kernel install-bin-lib install-wrappers \
	install-hoon-tools compile-kernel kernel compute-kernel-hash clean-kernel force-kernel

# Install hoonc + nockup from nocktoshi/nockchain when not already on PATH.
install-hoon-tools:
	@set -e; \
	if command -v hoonc >/dev/null 2>&1; then \
	  echo "✅ hoonc already installed ($$(command -v hoonc))"; \
	else \
	  echo "Installing hoonc ($(NOCKCHAIN_GIT)@$(NOCKCHAIN_BRANCH))..."; \
	  cargo +nightly install --git $(NOCKCHAIN_GIT) --branch $(NOCKCHAIN_BRANCH) --locked hoonc; \
	fi; \
	if command -v nockup >/dev/null 2>&1; then \
	  echo "✅ nockup already installed ($$(command -v nockup))"; \
	else \
	  echo "Installing nockup ($(NOCKCHAIN_GIT)@$(NOCKCHAIN_BRANCH))..."; \
	  cargo +nightly install --git $(NOCKCHAIN_GIT) --branch $(NOCKCHAIN_BRANCH) --locked nockup; \
	fi

$(NOCKUP_INSTALLED): nockapp.toml install-hoon-tools
	@echo "nockup package install..."
	nockup install
	@mkdir -p hoon/packages
	@touch $@

# Full install: hoon toolchain (if needed), kernel, Rust binary, wrappers.
install: install-hoon-tools install-kernel install-bin-lib install-wrappers

# Rust only: skip nockup + hoonc. Uses existing ./nns.jam (run `make install-kernel`
# or full `make install` when the kernel changes).
install-rust: install-bin-lib install-wrappers

compile-kernel: $(NOCKUP_INSTALLED)
	@set -e; \
	mkdir -p $(KERNEL_CACHE); \
	hash=$$($(MAKE) -s compute-kernel-hash); \
	if [ -f $(KERNEL_JAM) ] && [ -f $(KERNEL_BUILT_HASH) ] && [ "$$hash" = "$$(cat $(KERNEL_BUILT_HASH))" ]; then \
	  echo "✅ Kernel up to date ($(KERNEL_JAM))"; \
	else \
	  echo "Compiling Hoon kernel ($(KERNEL_JAM))..."; \
	  tmp="$(KERNEL_JAM).tmp"; \
	  log="$(KERNEL_CACHE)/hoonc.log"; \
	  rm -f "$$tmp"; \
	  TRACY_NO_INVARIANT_CHECK=1 hoonc --new hoon/app/app.hoon hoon/ --output "$$tmp" >"$$log" 2>&1 || true; \
	  if grep -qE 'Caught panic!|Error initializing NockApp:|missing dependency|fatal:' "$$log"; then \
	    echo "❌ Hoon kernel compile failed (see $$log)" >&2; \
	    cat "$$log" >&2; \
	    rm -f "$$tmp"; \
	    exit 1; \
	  fi; \
	  if [ ! -s "$$tmp" ]; then \
	    echo "❌ hoonc did not produce $(KERNEL_JAM) (see $$log)" >&2; \
	    cat "$$log" >&2; \
	    rm -f "$$tmp"; \
	    exit 1; \
	  fi; \
	  mv "$$tmp" "$(KERNEL_JAM)"; \
	  echo "$$hash" > "$(KERNEL_BUILT_HASH)"; \
	  echo "✅ Compiled $(KERNEL_JAM)"; \
	fi

compute-kernel-hash:
	@{ \
	  cat nockapp.toml; \
	  find hoon/app hoon/common hoon/dat hoon/lib hoon/jams -type f 2>/dev/null | sort | while IFS= read -r f; do cat "$$f"; done; \
	} | shasum -a 256 | awk '{print $$1}'

install-kernel: install-hoon-tools compile-kernel
	@test -s "$(KERNEL_JAM)" || { echo "❌ missing $(KERNEL_JAM); kernel compile failed" >&2; exit 1; }
	@echo "Installing Hoon kernel..."
	install -d "$(DESTDIR)$(LIBDIR)"
	install -m 644 "$(KERNEL_JAM)" "$(DESTDIR)$(KERNEL)"
	@echo "✅ Installed Hoon kernel to $(KERNEL)"

kernel: compile-kernel

force-kernel:
	@rm -f $(KERNEL_JAM) $(KERNEL_BUILT_HASH)
	@$(MAKE) compile-kernel

clean-kernel:
	rm -f $(KERNEL_JAM) $(KERNEL_BUILT_HASH) $(NOCKUP_INSTALLED)
	rm -rf hoon/common hoon/dat hoon/jams hoon/lib hoon/sur

install-bin-lib:
	cargo +nightly build --release
	install -d "$(DESTDIR)$(BINDIR)" "$(DESTDIR)$(LIBDIR)"
	install -m 755 "target/release/nns" "$(DESTDIR)$(BIN)"
	@echo "✅ Installed Rust binary to $(DESTDIR)$(BIN)"

install-wrappers:
	printf '#!/usr/bin/env sh\nexport TRACY_NO_INVARIANT_CHECK=1\nexport NNS_KERNEL_JAM=%s/nns.jam\nexec %s "$$@"\n' \
	  "$(LIBDIR)" "$(BIN)" > "$(DESTDIR)$(WRAPPER)"
	chmod 755 "$(DESTDIR)$(WRAPPER)"
	@touch "$(SHELL_RC)"; \
	if grep -qF '>>> nns installer >>>' "$(SHELL_RC)" 2>/dev/null; then \
	  awk '/^# >>> nns installer >>>$$/ { skip=1; next } \
	       /^# <<< nns installer <<<$$/ { skip=0; next } \
	       !skip { print }' "$(SHELL_RC)" > "$(SHELL_RC).nns.tmp" \
	    && mv "$(SHELL_RC).nns.tmp" "$(SHELL_RC)"; \
	fi; \
	grep -vF 'export PATH="$$HOME/.local/bin:$$PATH"' "$(SHELL_RC)" \
	  | grep -vF '; END=' > "$(SHELL_RC).nns.tmp" \
	  && mv "$(SHELL_RC).nns.tmp" "$(SHELL_RC)"; \
	printf '\n%s\n%s\n%s\n' \
	  '# >>> nns installer >>>' \
	  'export PATH="$$HOME/.local/bin:$$PATH"' \
	  '# <<< nns installer <<<' >> "$(SHELL_RC)"; \
	hash -r 2>/dev/null || true
	@printf '\n✅ Added to path: PATH="$$HOME/.local/bin:$$PATH"'
	@printf '\n\033[33m    Open a new shell to get "nns" commands.\033[0m\n'
	@printf '\n✅ Installed ℕℕ𝕊 CLI:'
	@printf '\n   Location: %s' "$(DESTDIR)$(WRAPPER)"
	@printf '\n   Command: nns --version\n\n'

uninstall:
	rm -f "$(DESTDIR)$(WRAPPER)"
	rm -f "$(DESTDIR)$(BIN)" "$(DESTDIR)$(KERNEL)"
	rmdir "$(DESTDIR)$(LIBDIR)" 2>/dev/null || true
