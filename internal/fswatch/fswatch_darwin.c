//go:build darwin && cgo

#include <stdlib.h>
#include <CoreServices/CoreServices.h>
#include <dispatch/dispatch.h>
#include "fswatch_darwin.h"
#include "_cgo_export.h"

struct fswatchStream {
	FSEventStreamRef stream;
	dispatch_queue_t queue;
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

static void noop(void *ctx) { (void)ctx; }

fswatchStream *fswatchStart(uintptr_t handle, const char *root, double latency) {
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
	fswatchStream *s = malloc(sizeof *s);
	if (s == NULL) {
		FSEventStreamRelease(stream);
		return NULL;
	}
	s->stream = stream;
	s->queue = dispatch_queue_create("irori.fswatch", DISPATCH_QUEUE_SERIAL);
	FSEventStreamSetDispatchQueue(stream, s->queue);
	if (!FSEventStreamStart(stream)) {
		FSEventStreamInvalidate(stream);
		FSEventStreamRelease(stream);
		dispatch_release(s->queue);
		free(s);
		return NULL;
	}
	return s;
}

// fswatchStop ends the stream. Once it returns no callback is running or
// will run, so the Go side can drop its handle.
void fswatchStop(fswatchStream *s) {
	FSEventStreamStop(s->stream);
	// The queue is serial: an empty task behind any in-flight callback
	// returns only after that callback has finished.
	dispatch_sync_f(s->queue, NULL, noop);
	FSEventStreamInvalidate(s->stream);
	FSEventStreamRelease(s->stream);
	dispatch_release(s->queue);
	free(s);
}
