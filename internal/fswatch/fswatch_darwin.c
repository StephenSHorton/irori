//go:build darwin && cgo

#include <stdatomic.h>
#include <stdlib.h>
#include <CoreServices/CoreServices.h>
#include "fswatch_darwin.h"
#include "_cgo_export.h"

// The stream runs on a CFRunLoop of a thread that Go owns (a goroutine
// locked to it), not on a dispatch queue. Callbacks from GCD's worker
// threads would each make the Go runtime bind an extra M to that thread for
// good, which outlives the watcher; on our own thread the callback reuses
// the M already running there, and the thread ends with the watcher.
struct fswatchStream {
	FSEventStreamRef stream;
	CFRunLoopRef loop;
	atomic_int stop;
};

// trampoline hands FSEvents' callback to Go. Without
// kFSEventStreamCreateFlagUseCFTypes the paths are a C string array.
static void trampoline(ConstFSEventStreamRef stream, void *info, size_t n,
                       void *paths, const FSEventStreamEventFlags flags[],
                       const FSEventStreamEventId ids[]) {
	(void)stream;
	(void)ids;
	goFSEvents((uintptr_t)info, n, (char **)paths, (uint32_t *)flags);
}

fswatchStream *fswatchCreate(uintptr_t handle, const char *root, double latency) {
	CFStringRef path = CFStringCreateWithCString(NULL, root, kCFStringEncodingUTF8);
	if (path == NULL) {
		return NULL;
	}
	CFArrayRef paths = CFArrayCreate(NULL, (const void **)&path, 1, &kCFTypeArrayCallBacks);
	CFRelease(path);
	if (paths == NULL) {
		return NULL;
	}
	FSEventStreamContext ctx = {0, (void *)handle, NULL, NULL, NULL};
	FSEventStreamRef stream = FSEventStreamCreate(NULL, trampoline, &ctx, paths,
		kFSEventStreamEventIdSinceNow, latency,
		kFSEventStreamCreateFlagFileEvents | kFSEventStreamCreateFlagNoDefer |
		kFSEventStreamCreateFlagWatchRoot);
	CFRelease(paths);
	if (stream == NULL) {
		return NULL;
	}
	fswatchStream *s = calloc(1, sizeof *s);
	if (s == NULL) {
		FSEventStreamRelease(stream);
		return NULL;
	}
	s->stream = stream;
	atomic_init(&s->stop, 0);
	return s;
}

int fswatchStart(fswatchStream *s) {
	s->loop = (CFRunLoopRef)CFRetain(CFRunLoopGetCurrent());
	FSEventStreamScheduleWithRunLoop(s->stream, s->loop, kCFRunLoopDefaultMode);
	if (!FSEventStreamStart(s->stream)) {
		FSEventStreamInvalidate(s->stream);
		FSEventStreamRelease(s->stream);
		s->stream = NULL;
		return 0;
	}
	return 1;
}

void fswatchLoop(fswatchStream *s) {
	// The timeout only bounds how late a stop signalled before the loop
	// started running is noticed; CFRunLoopStop ends a running wait at once.
	while (!atomic_load(&s->stop)) {
		CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.25, false);
	}
	FSEventStreamStop(s->stream);
	FSEventStreamInvalidate(s->stream);
	FSEventStreamRelease(s->stream);
	s->stream = NULL;
}

void fswatchSignalStop(fswatchStream *s) {
	atomic_store(&s->stop, 1);
	if (s->loop != NULL) {
		CFRunLoopStop(s->loop);
		CFRunLoopWakeUp(s->loop);
	}
}

void fswatchFree(fswatchStream *s) {
	if (s->loop != NULL) {
		CFRelease(s->loop);
	}
	free(s);
}
