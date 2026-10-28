# Koda system-wide installation.
#
# The default prefix is /usr/local, so `sudo make install` puts the `koda`
# binary in /usr/local/bin. Distro packages and staged installs can override
# PREFIX/DESTDIR without rebuilding. `make` itself never needs root; only the
# install target writes outside the build tree.

PREFIX ?= /usr/local
DESTDIR ?=
BINDIR ?= $(PREFIX)/bin
DATADIR ?= $(PREFIX)/share

CARGO ?= cargo
INSTALL ?= install

BINARY := target/release/koda
DOCDIR := $(DATADIR)/doc/koda
LICENSEDIR := $(DATADIR)/licenses/koda

# There are no shell completions, man pages or desktop entries in the tree, so
# this target installs only the binary, the licence and the written docs.
DOCS := README.md CHANGELOG.md SECURITY.md $(wildcard docs/*.md)

.PHONY: all build install uninstall clean

all: build

build:
	$(CARGO) build --release --locked

install: build
	$(INSTALL) -Dm755 "$(BINARY)" "$(DESTDIR)$(BINDIR)/koda"
	$(INSTALL) -Dm644 LICENSE "$(DESTDIR)$(LICENSEDIR)/LICENSE"
	@for doc in $(DOCS); do \
		$(INSTALL) -Dm644 "$$doc" "$(DESTDIR)$(DOCDIR)/$$doc"; \
	done

uninstall:
	rm -f "$(DESTDIR)$(BINDIR)/koda"
	rm -rf "$(DESTDIR)$(DOCDIR)" "$(DESTDIR)$(LICENSEDIR)"

clean:
	$(CARGO) clean
