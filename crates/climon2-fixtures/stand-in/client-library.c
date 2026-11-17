/*
 * Test-only stand-in for the client library (libclimon2_client.dylib or .so),
 * never shipped. It is also the faulty-library fixture: xtask's release-check
 * tests and climon2-platform's loader tests compile it, exact or with one
 * defect chosen by a macro, to prove that each check refuses that defect.
 *
 * Exact, it returns a complete revision-1 description of release RELEASE, a
 * string literal. None of its eleven calls is implemented: each one aborts the
 * process, so a test process that survives proves nothing called them.
 *
 * Probe: RECORD_STARTUP=<path string literal> makes create write the startup's
 * limits to that file, one `<field> <value>` line each in header order, then
 * fail with CM1_INTERNAL_ERROR instead of aborting, so a test sees exactly what
 * a caller passes.
 *
 * Release-check defects: EXTRA_EXPORT and HOOK_MARKER.
 * Loader defects: NO_BOOTSTRAP (no climon2_client_get_interface export),
 * UNSUPPORTED (refuses revision 1), BOOTSTRAP_STATUS=<code> (answers revision
 * 1 with that status and no description), FAIL_FIRST_BOOTSTRAP (a contained
 * failure, CM1_INTERNAL_ERROR, on the first call only), UNCLEARED_FAILURE
 * (answers without clearing the output it was given), REVISION=<n> and
 * RESERVED=<n> (the
 * description's fields), NULL_RELEASE (no release text), INCOMPLETE_TABLE (no
 * snapshot entry), and any of the requirements INSTANCE_BYTES,
 * OWNER_STACK_BYTES, WORKER_STACK_BYTES, WORKERS, INSTANCE_DESCRIPTORS and
 * REQUIREMENTS_RESERVED.
 */
#include <stdio.h>
#include <stdlib.h>

#include "client-v1.h"

/* The calls ignore their arguments: they abort. */
#pragma GCC diagnostic ignored "-Wunused-parameter"

#ifndef REVISION
#define REVISION CM1_REVISION
#endif
#ifndef RESERVED
#define RESERVED 0
#endif
#ifndef INSTANCE_BYTES
#define INSTANCE_BYTES 1000000
#endif
#ifndef OWNER_STACK_BYTES
#define OWNER_STACK_BYTES 2097152
#endif
#ifndef WORKER_STACK_BYTES
#define WORKER_STACK_BYTES 1048576
#endif
#ifndef WORKERS
#define WORKERS 2
#endif
#ifndef INSTANCE_DESCRIPTORS
#define INSTANCE_DESCRIPTORS 3
#endif
#ifndef REQUIREMENTS_RESERVED
#define REQUIREMENTS_RESERVED 0
#endif

#ifdef RECORD_STARTUP
static const uint8_t recorded_code[] = "internal_error";
static const uint8_t recorded_message[] = "the stand-in recorded its startup";

static void record(const cm1_limits *limits)
{
    FILE *file = fopen(RECORD_STARTUP, "w");
    if (file == 0)
        return;
    fprintf(file, "client_bytes %llu\n", (unsigned long long)limits->client_bytes);
    fprintf(file, "request_bytes %llu\n", (unsigned long long)limits->request_bytes);
    fprintf(file, "event_bytes %llu\n", (unsigned long long)limits->event_bytes);
    fprintf(file, "progress_bytes %llu\n", (unsigned long long)limits->progress_bytes);
    fprintf(file, "drain_payload_bytes %llu\n", (unsigned long long)limits->drain_payload_bytes);
    fprintf(file, "request_records %llu\n", (unsigned long long)limits->request_records);
    fprintf(file, "event_records %llu\n", (unsigned long long)limits->event_records);
    fprintf(file, "progress_records %llu\n", (unsigned long long)limits->progress_records);
    fprintf(file, "deliveries %llu\n", (unsigned long long)limits->deliveries);
    fprintf(file, "operations %llu\n", (unsigned long long)limits->operations);
    fprintf(file, "chunk_bytes %llu\n", (unsigned long long)limits->chunk_bytes);
    fprintf(file, "drain_records %llu\n", (unsigned long long)limits->drain_records);
    fprintf(file, "wire_frame_bytes %llu\n", (unsigned long long)limits->wire_frame_bytes);
    fprintf(file, "metadata_record_bytes %llu\n", (unsigned long long)limits->metadata_record_bytes);
    fprintf(file, "title_bytes %llu\n", (unsigned long long)limits->title_bytes);
    fprintf(file, "notification_title_bytes %llu\n", (unsigned long long)limits->notification_title_bytes);
    fprintf(file, "notification_body_bytes %llu\n", (unsigned long long)limits->notification_body_bytes);
    fprintf(file, "max_workers %llu\n", (unsigned long long)limits->max_workers);
    fprintf(file, "operation_ms %llu\n", (unsigned long long)limits->operation_ms);
    fprintf(file, "kill_ms %llu\n", (unsigned long long)limits->kill_ms);
    fprintf(file, "restoration_stall_ms %llu\n", (unsigned long long)limits->restoration_stall_ms);
    fprintf(file, "restoration_total_ms %llu\n", (unsigned long long)limits->restoration_total_ms);
    fprintf(file, "delivery_stall_ms %llu\n", (unsigned long long)limits->delivery_stall_ms);
    fprintf(file, "leave_ms %llu\n", (unsigned long long)limits->leave_ms);
    fprintf(file, "cleanup_ms %llu\n", (unsigned long long)limits->cleanup_ms);
    fprintf(file, "start_ms %llu\n", (unsigned long long)limits->start_ms);
    fprintf(file, "reserved %llu\n", (unsigned long long)limits->reserved);
    fclose(file);
}

static cm1_call stand_in_create(const cm1_startup *startup, cm1_created *out)
{
    record(&startup->limits);
    cm1_call call = {
        .code = CM1_INTERNAL_ERROR,
        .error = {
            .origin = CM1_ORIGIN_LIBRARY,
            .code = {recorded_code, sizeof recorded_code - 1},
            .message = {recorded_message, sizeof recorded_message - 1},
        },
    };
    return call;
}
#else
static cm1_call stand_in_create(const cm1_startup *startup, cm1_created *out) { abort(); }
#endif
static cm1_call stand_in_run(cm1_instance *instance, cm1_run_report *out) { abort(); }
static cm1_call stand_in_destroy(cm1_instance *instance) { abort(); }
static cm1_call stand_in_submit(cm1_communication *communication,
                                const cm1_request *request) { abort(); }
static cm1_call stand_in_drain(cm1_communication *communication, cm1_drained *out) { abort(); }
static cm1_call stand_in_release_delivery(cm1_delivery *delivery) { abort(); }
static cm1_call stand_in_shutdown(cm1_control *control) { abort(); }
static cm1_call stand_in_release_communication(cm1_communication *communication) { abort(); }
static cm1_call stand_in_release_control(cm1_control *control) { abort(); }
static cm1_call stand_in_notification(cm1_communication *communication, int32_t *out) { abort(); }
#ifdef INCOMPLETE_TABLE
#define SNAPSHOT 0
#else
static cm1_call stand_in_snapshot(cm1_communication *communication, cm1_metrics *out) { abort(); }
#define SNAPSHOT stand_in_snapshot
#endif

#ifdef NULL_RELEASE
#define RELEASE_TEXT {0, 0}
#else
static const uint8_t release[] = RELEASE;
#define RELEASE_TEXT {release, sizeof release - 1}
#endif

__attribute__((used)) static const cm1_description description = {
    .revision = REVISION,
    .reserved = RESERVED,
    .release = RELEASE_TEXT,
    .requirements = {
        .release_bytes = 5000000,
        .instance_bytes = INSTANCE_BYTES,
        .owner_stack_bytes = OWNER_STACK_BYTES,
        .worker_stack_bytes = WORKER_STACK_BYTES,
        .workers = WORKERS,
        .instance_descriptors = INSTANCE_DESCRIPTORS,
        .load_descriptors = 4,
        .reserved = REQUIREMENTS_RESERVED,
    },
    .functions = {
        .create = stand_in_create,
        .run = stand_in_run,
        .destroy = stand_in_destroy,
        .submit = stand_in_submit,
        .drain = stand_in_drain,
        .release_delivery = stand_in_release_delivery,
        .shutdown = stand_in_shutdown,
        .release_communication = stand_in_release_communication,
        .release_control = stand_in_release_control,
        .notification = stand_in_notification,
        .snapshot = SNAPSHOT,
    },
};

#ifdef NO_BOOTSTRAP
#define BOOTSTRAP climon2_client_get_interface_missing
#else
#define BOOTSTRAP climon2_client_get_interface
#endif

uint32_t BOOTSTRAP(uint32_t revision, const cm1_description **out)
{
    if (out == 0)
        return CM1_INVALID_ARGUMENT;
#ifndef UNCLEARED_FAILURE
    *out = 0;
#endif
#if defined(UNSUPPORTED)
    return CM1_UNSUPPORTED_REVISION;
#elif defined(BOOTSTRAP_STATUS)
    return BOOTSTRAP_STATUS;
#else
#ifdef FAIL_FIRST_BOOTSTRAP
    static int answered;
    if (!answered++)
        return CM1_INTERNAL_ERROR;
#endif
    if (revision != CM1_REVISION)
        return CM1_UNSUPPORTED_REVISION;
    *out = &description;
    return CM1_SUCCESS;
#endif
}

#ifdef EXTRA_EXPORT
uint32_t climon2_client_extra_export(void)
{
    return 0;
}
#endif

#ifdef HOOK_MARKER
__attribute__((used)) static const char marker[] = "CLIMON2_TEST_HOOKS_ENABLED";
#endif
