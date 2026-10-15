/* Minimal hello-world source. The pinned-real
 * fixture is a real-format Debian source tree, so
 * a real C source file is checked in. The test
 * never compiles it — the §13 inspection suite
 * reads the source tree but does not build it —
 * so the body is a one-line printf. */

#include <stdio.h>

int main(void) {
    puts("hello, world");
    return 0;
}
