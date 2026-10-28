// FSEvents glue for fswatch_darwin.go; see fswatch_darwin.c.
#include <stddef.h>
#include <stdint.h>

typedef struct fswatchStream fswatchStream;

// fswatchCreate makes a stream for root; NULL on failure. Nothing runs yet.
fswatchStream *fswatchCreate(uintptr_t handle, const char *root, double latency);
// fswatchStart schedules the stream on the calling thread's run loop and
// starts it; 0 on failure (the stream is then released).
int fswatchStart(fswatchStream *s);
// fswatchLoop runs the calling thread's run loop until fswatchSignalStop,
// then stops, invalidates and releases the stream. Callbacks run only in
// here, on this thread.
void fswatchLoop(fswatchStream *s);
// fswatchSignalStop asks fswatchLoop to return; safe from any thread.
void fswatchSignalStop(fswatchStream *s);
void fswatchFree(fswatchStream *s);
