PREFIX ?= $(HOME)/.local
BINDIR := $(PREFIX)/bin
LIBDIR := $(PREFIX)/lib/nns
WRAPPER := $(BINDIR)/nns
BIN := $(LIBDIR)/nns
KERNEL := $(LIBDIR)/nns.jam
SHELL_RC ?= $(HOME)/.zshrc
PATH_LINE := export PATH="$$HOME/.local/bin:$$PATH"

VESL_LIB_NAMES := vesl-graft vesl-merkle vesl-prover vesl-stark-verifier vesl-verifier

.PHONY: install install-rust uninstall install-kernel sync-hoon-from-nockup install-bin-lib install-wrappers

# Copy nockup caches into canonical hoon/ layout (real files, no symlinks).
# Sources of truth: hoon/packages/*; compile tree: hoon/common, hoon/dat, hoon/lib.
sync-hoon-from-nockup:
	@nock_pkg=$$(ls -d hoon/packages/nockchain-hoon--commit-* 2>/dev/null | head -1); \
	vesl_pkg=$$(ls -d hoon/packages/vesl-lib--commit-* 2>/dev/null | head -1); \
	if [ -z "$$nock_pkg" ] || [ -z "$$vesl_pkg" ]; then \
	  echo "missing hoon/packages/*; run: nockup package install" >&2; \
	  exit 1; \
	fi; \
	mkdir -p hoon/common hoon/dat hoon/jams hoon/lib; \
	for f in $(VESL_LIB_NAMES); do rm -f "hoon/lib/$$f.hoon"; done; \
	rsync -a --delete "$$nock_pkg/common/" hoon/common/; \
	rsync -a --delete "$$nock_pkg/dat/" hoon/dat/; \
	rsync -a --delete "$$nock_pkg/jams/" hoon/jams/; \
	for f in $(VESL_LIB_NAMES); do \
	  cp "$$vesl_pkg/$$f.hoon" "hoon/lib/$$f.hoon"; \
	done; \
	test -f hoon/common/wrapper.hoon || cp "$$nock_pkg/common/wrapper.hoon" hoon/common/wrapper.hoon; \
	echo "Materialized hoon/common, hoon/dat, hoon/jams, hoon/lib from hoon/packages/"

# Full install: compile Hoon kernel (nns.jam) then Rust release binary + wrappers.
install: install-kernel install-bin-lib install-wrappers

# Rust only: skip nockup + hoonc. Uses existing ./nns.jam (run `make install-kernel`
# or full `make install` when the kernel changes).
install-rust: install-bin-lib install-wrappers

install-kernel:
	@echo "Installing Hoon kernel (nns.jam)..."
	nockup install
	$(MAKE) sync-hoon-from-nockup
	TRACY_NO_INVARIANT_CHECK=1 hoonc --new hoon/app/app.hoon hoon/ --output nns.jam
	install -d "$(DESTDIR)$(LIBDIR)"
	install -m 644 "nns.jam" "$(DESTDIR)$(KERNEL)"
	rm -rf hoon/common hoon/dat hoon/jams hoon/lib hoon/sur

install-bin-lib:
	cargo +nightly build --release
	install -d "$(DESTDIR)$(BINDIR)" "$(DESTDIR)$(LIBDIR)"
	install -m 755 "target/release/nns" "$(DESTDIR)$(BIN)"

install-wrappers:
	printf '#!/usr/bin/env sh\nexport TRACY_NO_INVARIANT_CHECK=1\nexport NNS_KERNEL_JAM=%s/nns.jam\nexec %s/nns "$$@"\n' \
	  "$(LIBDIR)" "$(BIN)" > "$(DESTDIR)$(WRAPPER)"
	chmod 755 "$(DESTDIR)$(WRAPPER)"
	@touch "$(SHELL_RC)"
	@grep -qxF '$(PATH_LINE)' "$(SHELL_RC)" 2>/dev/null || printf '\n%s\n' '$(PATH_LINE)' >> "$(SHELL_RC)"
	@hash -r 2>/dev/null || true
	@printf '\nInstalled NNS CLI:\n'
	@printf '  %s\n' "$(DESTDIR)$(WRAPPER)"
	@printf '\nUpdated %s with:\n' "$(SHELL_RC)"
	@printf '  %s\n' '$(PATH_LINE)'
	@printf 'Open a new shell if `nns` still resolves to another tool.\n'

uninstall:
	rm -f "$(DESTDIR)$(WRAPPER)"
	rm -f "$(DESTDIR)$(BIN)" "$(DESTDIR)$(KERNEL)"
	rmdir "$(DESTDIR)$(LIBDIR)" 2>/dev/null || true
