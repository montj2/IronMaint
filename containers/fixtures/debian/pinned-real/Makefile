# Minimal Makefile for the pinned-real fixture. The
# §13 inspection suite reads but does not build this
# tree, so the Makefile is informational only — it
# records the canonical `hello` upstream's build
# pattern so the source tree looks like a real
# Debian package (a hand-crafted-but-realistic
# fixture). The Makefile must compile hello.c
# against libc only, no extra dependencies.

CC ?= cc
CFLAGS ?= -O2 -Wall -Wextra

hello: hello.c
	$(CC) $(CFLAGS) -o $@ $<

.PHONY: clean
clean:
	rm -f hello
