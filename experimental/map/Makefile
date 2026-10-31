CC ?= cc
CFLAGS ?= -std=c23 -O2 -g -Wall -Wextra -Wpedantic -Wshadow -Wconversion -Wno-sign-conversion
# The readers of mapk.h are written in the style of Arthur Whitney, which these warn less about
KFLAGS ?= -std=c23 -O2 -g -Wall -Wno-parentheses -Wno-misleading-indentation
RUSTC ?= rustc
RUSTCFLAGS ?= --edition 2024 -C opt-level=2

# The repository root, and this directory from it: compare.sh takes paths from the root
ROOT = ../..
HERE = experimental/map

# `make debug` builds with the sanitizers. w64devkit has no sanitizer
# runtimes, so on Windows undefined behavior traps and there is no ASan.
# `make check` compares this reader with the Rust one in ../../src/map.rs, `make
# check2` map2.c, and `make kcheck` the four readers of mapk.h, which only say
# whether a map failed.
ifeq ($(OS),Windows_NT)
SANITIZE = -fsanitize=undefined -fsanitize-trap=all
RUST_TARGET = x86_64-pc-windows-gnu
RUST_DUMP = target/$(RUST_TARGET)/release/examples/reader-dump
CARGO_TARGET = --target $(RUST_TARGET)
EXE = .exe
else
SANITIZE = -fsanitize=address,undefined -fno-omit-frame-pointer
RUST_DUMP = target/release/examples/reader-dump
CARGO_TARGET =
EXE =
endif

# Rust links the GNU target with -lgcc_eh, which newer MinGW GCCs no longer ship;
# as ../../build.rs does for cargo, give it an empty one when GCC has none
ifeq ($(OS)$(shell $(CC) -print-file-name=libgcc_eh.a),Windows_NTlibgcc_eh.a)
GCC_EH = gcc_eh/libgcc_eh.a
RUSTC_LINK = -L gcc_eh
endif

KREADERS = ki kd kai kad

all: mapdump mapbench mapdump2 mapbench2 $(KREADERS:%=map%dump) $(KREADERS:%=map%bench) maptools

mapdump: mapdump.o map.o
	$(CC) $(CFLAGS) -o $@ $^

mapbench: mapbench.o map.o
	$(CC) $(CFLAGS) -o $@ $^

# map2.c is map.c with its arrays read at once
mapdump2: mapdump.o map2.o
	$(CC) $(CFLAGS) -o $@ $^

mapbench2: mapbench-c2.o map2.o
	$(CC) $(CFLAGS) -o $@ $^

mapbench-c2.o: mapbench.c map.h
	$(CC) $(CFLAGS) -DLABEL='"c2"' -c -o $@ $<

# mapki is the schema interpreter, mapkd the schemas as code, and mapkai and
# mapkad the same in assembly
mapk%dump: mapkdump.o mapk%.o
	$(CC) $(KFLAGS) -o $@ $^

mapk%bench: mapbench-k%.o mapk%.o
	$(CC) $(CFLAGS) -o $@ $^

%.o: %.c map.h
	$(CC) $(CFLAGS) -c -o $@ $<

mapk%.o: mapk%.c mapk.h
	$(CC) $(KFLAGS) -c -o $@ $<

mapk%.o: mapk%.S mapka.inc mapk.h
	$(CC) -c -o $@ $<

mapbench-%.o: mapbench.c mapk.h
	$(CC) $(CFLAGS) -DMAPK='"$*"' -c -o $@ $<

# The Rust readers in the style of map.c, each built on its own with rustc
maptools: maptool$(EXE) maptool2$(EXE) maptool3$(EXE)

maptool$(EXE): maptool.rs map.rs $(GCC_EH)
	$(RUSTC) $(RUSTCFLAGS) $(CARGO_TARGET) $(RUSTC_LINK) -o $@ maptool.rs

maptool2$(EXE): maptool.rs map2.rs $(GCC_EH)
	$(RUSTC) $(RUSTCFLAGS) $(CARGO_TARGET) $(RUSTC_LINK) --cfg map2 -o $@ maptool.rs

maptool3$(EXE): maptool3.rs map3.rs $(GCC_EH)
	$(RUSTC) $(RUSTCFLAGS) $(CARGO_TARGET) $(RUSTC_LINK) -C panic=abort -C lto=fat -C codegen-units=1 -o $@ maptool3.rs

gcc_eh/libgcc_eh.a:
	mkdir -p gcc_eh
	printf '!<arch>\n' > $@

debug: CFLAGS += $(SANITIZE)
debug: KFLAGS += $(SANITIZE)
debug: clean mapdump mapkidump mapkddump

check: mapdump
	cd $(ROOT) && cargo build --quiet --release $(CARGO_TARGET) --example reader-dump
	sh compare.sh $(HERE)/mapdump $(RUST_DUMP)

check2: mapdump2
	cd $(ROOT) && cargo build --quiet --release $(CARGO_TARGET) --example reader-dump
	sh compare.sh $(HERE)/mapdump2 $(RUST_DUMP)

kcheck: $(KREADERS:%=map%dump)
	cd $(ROOT) && cargo build --quiet --release $(CARGO_TARGET) --example reader-dump
	for k in $(KREADERS); do echo "map$$k:" && sh compare.sh --status $(HERE)/map$${k}dump $(RUST_DUMP) || exit 1; done

clean:
	rm -f mapdump mapdump.exe mapbench mapbench.exe mapdump2 mapdump2.exe mapbench2 mapbench2.exe mapk*dump mapk*dump.exe mapk*bench mapk*bench.exe *.o
	rm -f maptool maptool.exe maptool2 maptool2.exe maptool3 maptool3.exe *.pdb
	rm -rf gcc_eh

.SECONDARY: $(KREADERS:%=map%.o) $(KREADERS:%=mapbench-%.o)
.PHONY: all maptools debug check check2 kcheck clean
