#include "client-v1.h"
#include <string.h>

cm1_limits example_limits(void)
{
    cm1_limits limits = {
        .client_bytes = 33554432,
        .request_bytes = 262144,
        .event_bytes = 1048576,
        .progress_bytes = 65536,
        .drain_payload_bytes = 65536,
        .request_records = 64,
        .event_records = 1024,
        .progress_records = 32,
        .deliveries = 8,
        .operations = 16,
        .chunk_bytes = 65536,
        .drain_records = 64,
        .wire_frame_bytes = 1048576,
        .metadata_record_bytes = 16384,
        .title_bytes = 4096,
        .notification_title_bytes = 4096,
        .notification_body_bytes = 8192,
        .max_workers = 2,
        .operation_ms = 5000,
        .kill_ms = 10000,
        .restoration_stall_ms = 5000,
        .restoration_total_ms = 60000,
        .delivery_stall_ms = 5000,
        .leave_ms = 5000,
        .cleanup_ms = 10000,
        .start_ms = 10000
    };
    return limits;
}

cm1_call example_bootstrap(cm1_bytes expected, const cm1_description **out)
{
    uint32_t status = climon2_client_get_interface(CM1_REVISION, out);
    if (status != CM1_SUCCESS)
        return (cm1_call){.code = status,
            .error = {CM1_ORIGIN_LIBRARY, 0,
                {(const uint8_t *)"bootstrap_failed", 16},
                {(const uint8_t *)"Interface lookup failed", 23}}};
    if ((*out)->revision != CM1_REVISION ||
        (*out)->release.length != expected.length ||
        memcmp((*out)->release.data, expected.data, expected.length) != 0) {
        *out = 0;
        return (cm1_call){.code = CM1_RELEASE_MISMATCH,
            .error = {CM1_ORIGIN_LIBRARY, 0,
                {(const uint8_t *)"release_mismatch", 16},
                {(const uint8_t *)"Exact release validation failed", 31}}};
    }
    return (cm1_call){0};
}

cm1_call example_create(const cm1_interface *api, cm1_created *out)
{
    static const uint8_t release[] = "0.1.0";
    static const uint8_t metadata[] = "/tmp/climon2-home/sessions/session.json";
    static const uint8_t home[] = "/tmp/climon2-home";
    static const uint8_t runtime[] = "/tmp/climon2-runtime";
    static const uint8_t filter[] = "info";
    cm1_startup startup = {0};
    startup.session_id = (cm1_guid){{0x00, 0x11, 0x22, 0x33, 0x44, 0x55,
        0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff}};
    startup.generation = 1;
    startup.expected_release = (cm1_bytes){release, sizeof release - 1};
    startup.metadata_path = (cm1_path){
        CM1_NATIVE_UNIX, 0, {metadata, sizeof metadata - 1}};
    startup.home_path = (cm1_path){
        CM1_NATIVE_UNIX, 0, {home, sizeof home - 1}};
    startup.runtime_path = (cm1_path){
        CM1_NATIVE_UNIX, 0, {runtime, sizeof runtime - 1}};
    startup.settings = (cm1_settings){1, 29, 1, 20, 0};
    startup.diagnostics.filter = (cm1_bytes){filter, sizeof filter - 1};
    startup.limits = example_limits();
    return api->create(&startup, out);
}

cm1_call example_open(const cm1_interface *api, cm1_communication *communication)
{
    static const uint8_t status[] = "status-v1";
    static const uint8_t title[] = "title-v1";
    static const uint8_t focus[] = "focus-v1";
    cm1_bytes required[] = {{status, sizeof status - 1}};
    cm1_bytes optional[] = {
        {title, sizeof title - 1}, {focus, sizeof focus - 1}};
    cm1_request request = {0};
    request.tag = CM1_OPEN;
    request.request_id = 1;
    request.args.open.mode = CM1_MONITOR;
    request.args.open.required = (cm1_tokens){required, 1};
    request.args.open.optional = (cm1_tokens){optional, 2};
    return api->submit(communication, &request);
}

cm1_call example_input(const cm1_interface *api, cm1_communication *communication,
                      uint64_t id, cm1_authority authority)
{
    const uint8_t bytes[] = {0x00, 0xff, 0x41};
    cm1_request request = {0};
    request.tag = CM1_INPUT;
    request.request_id = id;
    request.args.input = (cm1_input){authority, {bytes, sizeof bytes}};
    return api->submit(communication, &request);
}

cm1_call example_drain(const cm1_interface *api,
                      cm1_communication *communication, cm1_drained *out)
{
    return api->drain(communication, out);
}

cm1_call example_owner_finish(const cm1_interface *api, cm1_instance *instance,
                             cm1_run_report *report)
{
    return api->run(instance, report);
}

cm1_call example_destroy_after_release(const cm1_interface *api,
                                      cm1_instance *instance)
{
    return api->destroy(instance);
}

cm1_call example_release_batch(const cm1_interface *api, cm1_drained *drained)
{
    cm1_call result = api->release_delivery(drained->delivery);
    if (result.code == CM1_SUCCESS)
        *drained = (cm1_drained){0};
    return result;
}

cm1_call example_release_handles_after_terminal(const cm1_interface *api,
                                               cm1_created *created)
{
    cm1_call result = api->release_communication(created->communication);
    if (result.code != CM1_SUCCESS)
        return result;
    created->communication = 0;
    result = api->release_control(created->control);
    if (result.code == CM1_SUCCESS)
        created->control = 0;
    return result;
}

cm1_call example_shutdown(const cm1_interface *api, cm1_control *control)
{
    return api->shutdown(control);
}
