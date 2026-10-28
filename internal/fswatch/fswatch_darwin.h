// FSEvents glue for fswatch_darwin.go; see fswatch_darwin.c.
#include <stddef.h>
#include <stdint.h>

typedef struct fswatchStream fswatchStream;

fswatchStream *fswatchStart(uintptr_t handle, const char *root, double latency);
void fswatchStop(fswatchStream *s);
