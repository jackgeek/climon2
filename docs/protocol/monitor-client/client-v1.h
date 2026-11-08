#ifndef CLIMON2_CLIENT_V1_H
#define CLIMON2_CLIENT_V1_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define CM1_REVISION UINT32_C(1)
#define CM1_SUCCESS UINT32_C(0)
#define CM1_UNSUPPORTED_REVISION UINT32_C(1)
#define CM1_INVALID_ARGUMENT UINT32_C(2)
#define CM1_UNSUPPORTED_OPERATION UINT32_C(3)
#define CM1_INVALID_REQUEST_ID UINT32_C(4)
#define CM1_FULL UINT32_C(5)
#define CM1_CLOSED UINT32_C(6)
#define CM1_BUSY UINT32_C(7)
#define CM1_WRONG_THREAD UINT32_C(8)
#define CM1_UNSUPPORTED_LIMIT UINT32_C(9)
#define CM1_RESOURCE_LIMIT UINT32_C(10)
#define CM1_INTERNAL_ERROR UINT32_C(11)
#define CM1_CLEANUP_FAILED UINT32_C(12)
#define CM1_TIMEOUT UINT32_C(13)
#define CM1_RELEASE_MISMATCH UINT32_C(14)

#define CM1_ORIGIN_NONE UINT32_C(0)
#define CM1_ORIGIN_LIBRARY UINT32_C(1)
#define CM1_ORIGIN_DAEMON UINT32_C(2)
#define CM1_ORIGIN_TRANSPORT UINT32_C(3)

#define CM1_OK UINT32_C(1)
#define CM1_REJECTED UINT32_C(2)
#define CM1_FAILED UINT32_C(3)
#define CM1_INDETERMINATE UINT32_C(4)

#define CM1_MONITOR UINT32_C(1)
#define CM1_INTERACTIVE UINT32_C(2)
#define CM1_NATIVE_UNIX UINT32_C(1)
#define CM1_UNAVAILABLE UINT32_C(0)
#define CM1_UNSET UINT32_C(1)
#define CM1_VALUE UINT32_C(2)

#define CM1_IDLE UINT32_C(1)
#define CM1_RUNNING UINT32_C(2)
#define CM1_NEEDS_ATTENTION UINT32_C(3)
#define CM1_ACKNOWLEDGED UINT32_C(4)
#define CM1_COMPLETED UINT32_C(5)
#define CM1_STATUS_FAILED UINT32_C(6)
#define CM1_PROGRESS_NONE UINT32_C(1)
#define CM1_PROGRESS_NORMAL UINT32_C(2)
#define CM1_PROGRESS_INDETERMINATE UINT32_C(3)
#define CM1_PROGRESS_ERROR UINT32_C(4)
#define CM1_PROGRESS_PAUSED UINT32_C(5)
#define CM1_EXIT UINT32_C(1)
#define CM1_SIGNAL UINT32_C(2)
#define CM1_CLEANUP_COMPLETE UINT32_C(1)
#define CM1_CLEANUP_INCOMPLETE UINT32_C(2)
#define CM1_CLEANUP_UNKNOWN UINT32_C(3)

#define CM1_OPEN UINT32_C(1)
#define CM1_INPUT UINT32_C(2)
#define CM1_RESIZE UINT32_C(3)
#define CM1_ACQUIRE UINT32_C(4)
#define CM1_READY UINT32_C(5)
#define CM1_VIEWING UINT32_C(6)
#define CM1_ACKNOWLEDGE UINT32_C(7)
#define CM1_LEAVE UINT32_C(8)
#define CM1_KILL UINT32_C(9)
#define CM1_FOCUS UINT32_C(10)

#define CM1_RESULT_NONE UINT32_C(0)
#define CM1_OPENED UINT32_C(1)
#define CM1_INPUT_ACCEPTED UINT32_C(2)
#define CM1_RESIZE_ACCEPTED UINT32_C(3)
#define CM1_ACQUIRED UINT32_C(4)
#define CM1_READY_RESULT UINT32_C(5)
#define CM1_VIEWING_RESULT UINT32_C(6)
#define CM1_ACKNOWLEDGEMENT UINT32_C(7)
#define CM1_LEFT UINT32_C(8)
#define CM1_TERMINATION UINT32_C(9)
#define CM1_FOCUS_RESULT UINT32_C(10)
#define CM1_KNOWN_FINAL UINT32_C(11)

#define CM1_RESPONSE UINT32_C(1)
#define CM1_INITIAL_METADATA UINT32_C(2)
#define CM1_TITLE UINT32_C(3)
#define CM1_STATUS UINT32_C(4)
#define CM1_NOTIFICATION UINT32_C(5)
#define CM1_PROGRESS UINT32_C(6)
#define CM1_GRANTED UINT32_C(7)
#define CM1_REVOKED UINT32_C(8)
#define CM1_RESTORATION_BEGIN UINT32_C(9)
#define CM1_DISPLAY UINT32_C(10)
#define CM1_RESTORATION_COMPLETE UINT32_C(11)
#define CM1_FINAL UINT32_C(12)
#define CM1_GEOMETRY UINT32_C(13)
#define CM1_DIAGNOSTIC UINT32_C(14)

#define CM1_ATTACH UINT32_C(1)
#define CM1_TAKE_CONTROL UINT32_C(2)
#define CM1_HAND_BACK UINT32_C(3)
#define CM1_SUCCESSOR UINT32_C(4)
#define CM1_TAKEOVER UINT32_C(1)
#define CM1_SESSION_ENDING UINT32_C(2)
#define CM1_OSC9 UINT32_C(1)
#define CM1_OSC99 UINT32_C(2)
#define CM1_OSC777 UINT32_C(3)

#define CM1_DELIVERY UINT32_C(1)
#define CM1_EMPTY UINT32_C(2)
#define CM1_TERMINAL UINT32_C(3)
#define CM1_CLIENT_STOPPED UINT32_C(1)
#define CM1_CLIENT_FAILED UINT32_C(2)
#define CM1_RETRY_NONE UINT32_C(1)
#define CM1_RETRY_PASSIVE UINT32_C(2)
#define CM1_RETRY_PERMANENT UINT32_C(3)
#define CM1_LEAVE_NOT_REQUIRED UINT32_C(1)
#define CM1_LEAVE_CONFIRMED UINT32_C(2)
#define CM1_LEAVE_REJECTED UINT32_C(3)
#define CM1_LEAVE_FAILED UINT32_C(4)
#define CM1_LEAVE_INDETERMINATE UINT32_C(5)

typedef struct cm1_instance cm1_instance;
typedef struct cm1_communication cm1_communication;
typedef struct cm1_control cm1_control;
typedef struct cm1_delivery cm1_delivery;

typedef struct { uint8_t bytes[16]; } cm1_guid;
typedef struct { uint8_t bytes[24]; } cm1_utc;
typedef struct { const uint8_t *data; uint64_t length; } cm1_bytes;
typedef struct { const cm1_bytes *data; uint64_t count; } cm1_tokens;
typedef struct {
    uint32_t present;
    uint32_t reserved;
    cm1_bytes value;
} cm1_optional_text;
typedef struct {
    uint32_t encoding;
    uint32_t reserved;
    cm1_bytes bytes;
} cm1_path;
typedef struct {
    uint32_t present;
    uint32_t reserved;
    cm1_path value;
} cm1_optional_path;
typedef struct { uint32_t cols; uint32_t rows; } cm1_geometry;
typedef struct {
    uint64_t generation;
    uint64_t period;
    uint64_t restoration_id;
} cm1_authority;
typedef struct {
    uint32_t origin;
    uint32_t reserved;
    cm1_bytes code;
    cm1_bytes message;
} cm1_error;
typedef struct {
    uint32_t code;
    uint32_t reserved;
    cm1_error error;
} cm1_call;
typedef struct {
    uint32_t state;
    uint32_t reserved;
    cm1_bytes value;
} cm1_title;
typedef struct {
    uint32_t state;
    uint32_t episode_present;
    uint64_t episode_id;
    cm1_utc changed_at;
} cm1_status;
typedef struct {
    uint32_t available;
    uint32_t reserved;
    cm1_status value;
} cm1_available_status;
typedef struct {
    uint32_t state;
    uint32_t percent_present;
    uint32_t percent;
    uint32_t reserved;
} cm1_progress;
typedef struct {
    uint32_t available;
    uint32_t reserved;
    cm1_progress value;
} cm1_available_progress;
typedef struct {
    uint32_t kind;
    uint32_t value;
    cm1_optional_text signal_name;
} cm1_command_result;
typedef struct {
    uint32_t present;
    uint32_t reserved;
    cm1_command_result value;
} cm1_optional_command;
typedef struct {
    cm1_title title;
    cm1_available_status status;
    cm1_available_progress progress;
    cm1_optional_command command_result;
} cm1_metadata;
typedef struct {
    uint8_t escape_enabled;
    uint8_t escape_byte;
    uint8_t take_control_enabled;
    uint8_t take_control_byte;
    uint32_t reserved;
} cm1_settings;
typedef struct {
    cm1_optional_path log_path;
    cm1_bytes filter;
} cm1_diagnostics;
typedef struct {
    uint64_t client_bytes;
    uint64_t request_bytes;
    uint64_t event_bytes;
    uint64_t progress_bytes;
    uint64_t drain_payload_bytes;
    uint32_t request_records;
    uint32_t event_records;
    uint32_t progress_records;
    uint32_t deliveries;
    uint32_t operations;
    uint32_t chunk_bytes;
    uint32_t drain_records;
    uint32_t wire_frame_bytes;
    uint32_t metadata_record_bytes;
    uint32_t title_bytes;
    uint32_t notification_title_bytes;
    uint32_t notification_body_bytes;
    uint32_t max_workers;
    uint32_t operation_ms;
    uint32_t kill_ms;
    uint32_t restoration_stall_ms;
    uint32_t restoration_total_ms;
    uint32_t delivery_stall_ms;
    uint32_t leave_ms;
    uint32_t cleanup_ms;
    uint32_t start_ms;
    uint32_t reserved;
} cm1_limits;
typedef struct {
    uint64_t release_bytes;
    uint64_t instance_bytes;
    uint64_t owner_stack_bytes;
    uint64_t worker_stack_bytes;
    uint32_t workers;
    uint32_t instance_descriptors;
    uint32_t load_descriptors;
    uint32_t reserved;
} cm1_requirements;
typedef struct {
    cm1_guid session_id;
    uint64_t generation;
    cm1_bytes expected_release;
    cm1_path metadata_path;
    cm1_path home_path;
    cm1_path runtime_path;
    cm1_settings settings;
    cm1_diagnostics diagnostics;
    cm1_limits limits;
} cm1_startup;
typedef struct {
    cm1_instance *instance;
    cm1_communication *communication;
    cm1_control *control;
    cm1_limits effective_limits;
} cm1_created;
typedef struct {
    uint32_t mode;
    uint32_t reserved;
    cm1_geometry geometry;
    cm1_tokens required;
    cm1_tokens optional;
} cm1_open;
typedef struct { cm1_authority authority; cm1_bytes bytes; } cm1_input;
typedef struct {
    cm1_authority authority;
    cm1_geometry geometry;
} cm1_resize;
typedef struct {
    cm1_authority authority;
    uint8_t value;
    uint8_t reserved[7];
} cm1_flag;
typedef struct {
    uint32_t tag;
    uint32_t reserved;
    uint64_t request_id;
    union {
        cm1_open open;
        cm1_input input;
        cm1_resize resize;
        uint64_t generation;
        cm1_authority authority;
        cm1_flag flag;
        cm1_tokens required;
    } args;
} cm1_request;
typedef struct {
    cm1_guid connection_id;
    cm1_tokens selected;
} cm1_opened;
typedef struct {
    uint8_t changed;
    uint8_t reserved[7];
    uint64_t period;
    uint64_t restoration_id;
} cm1_authority_result;
typedef struct {
    uint8_t changed;
    uint8_t value;
    uint8_t reserved[2];
    uint32_t accepted_bytes;
} cm1_flag_result;
typedef struct {
    cm1_optional_command command_result;
    uint32_t cleanup;
    uint32_t reserved;
} cm1_termination;
typedef struct {
    uint64_t request_id;
    uint32_t outcome;
    uint32_t result_tag;
    cm1_error error;
    union {
        cm1_opened opened;
        uint32_t accepted_bytes;
        cm1_authority_result authority;
        cm1_flag_result flag;
        uint8_t changed;
        uint8_t attachment_detached;
        cm1_termination termination;
        cm1_command_result known_final;
    } result;
} cm1_response;
typedef struct {
    uint64_t period;
    uint64_t restoration_id;
    cm1_geometry geometry;
    uint32_t reason;
    uint32_t reserved;
} cm1_granted;
typedef struct {
    uint64_t period;
    uint64_t restoration_id;
    uint32_t reason;
    uint32_t reserved;
} cm1_revoked;
typedef struct {
    uint64_t period;
    uint64_t restoration_id;
    cm1_geometry geometry;
} cm1_restoration;
typedef struct { uint64_t period; uint64_t restoration_id; } cm1_barrier;
typedef struct { cm1_barrier authority; cm1_bytes bytes; } cm1_display;
typedef struct { uint64_t period; cm1_geometry geometry; } cm1_geometry_event;
typedef struct {
    uint32_t source;
    uint32_t episode_present;
    uint64_t episode_id;
    cm1_optional_text source_id;
    cm1_optional_text title;
    cm1_optional_text body;
} cm1_notification;
typedef struct {
    cm1_command_result command_result;
    cm1_available_status status;
} cm1_final;
typedef struct {
    uint32_t tag;
    uint32_t reserved;
    uint64_t generation;
    union {
        cm1_response response;
        cm1_metadata metadata;
        cm1_title title;
        cm1_status status;
        cm1_notification notification;
        cm1_progress progress;
        cm1_granted granted;
        cm1_revoked revoked;
        cm1_restoration restoration;
        cm1_display display;
        cm1_barrier barrier;
        cm1_geometry_event geometry;
        cm1_final final;
        cm1_error diagnostic;
    } data;
} cm1_record;
typedef struct {
    uint64_t generation;
    uint32_t client_outcome;
    uint32_t retry;
    uint32_t leave;
    uint8_t attachment_detached;
    uint8_t connection_closed;
    uint8_t reserved[2];
    cm1_error error;
} cm1_terminal;
typedef struct {
    uint32_t workers_cleanup;
    uint32_t reserved;
    cm1_terminal terminal;
} cm1_run_report;
typedef struct {
    uint32_t kind;
    uint32_t reserved;
    cm1_delivery *delivery;
    const cm1_record *records;
    uint64_t count;
    const cm1_terminal *terminal;
} cm1_drained;
typedef struct {
    uint64_t allocation_bytes;
    uint64_t peak_allocation_bytes;
    uint64_t release_allocation_bytes;
    uint64_t peak_release_allocation_bytes;
    uint64_t request_bytes;
    uint64_t peak_request_bytes;
    uint64_t event_bytes;
    uint64_t peak_event_bytes;
    uint64_t input_bytes_accepted;
    uint64_t display_bytes_delivered;
    uint64_t records_drained;
    uint64_t deliveries_released;
    uint32_t request_records;
    uint32_t peak_request_records;
    uint32_t event_records;
    uint32_t peak_event_records;
    uint32_t deliveries;
    uint32_t peak_deliveries;
    uint32_t operations;
    uint32_t peak_operations;
    uint32_t workers;
    uint32_t peak_workers;
    uint32_t descriptors;
    uint32_t peak_descriptors;
} cm1_metrics;

typedef struct {
    cm1_call (*create)(const cm1_startup *, cm1_created *);
    cm1_call (*run)(cm1_instance *, cm1_run_report *);
    cm1_call (*destroy)(cm1_instance *);
    cm1_call (*submit)(cm1_communication *, const cm1_request *);
    cm1_call (*drain)(cm1_communication *, cm1_drained *);
    cm1_call (*release_delivery)(cm1_delivery *);
    cm1_call (*shutdown)(cm1_control *);
    cm1_call (*release_communication)(cm1_communication *);
    cm1_call (*release_control)(cm1_control *);
    cm1_call (*notification)(cm1_communication *, int32_t *);
    cm1_call (*snapshot)(cm1_communication *, cm1_metrics *);
} cm1_interface;
typedef struct {
    uint32_t revision;
    uint32_t reserved;
    cm1_bytes release;
    cm1_tokens capabilities;
    cm1_requirements requirements;
    cm1_interface functions;
} cm1_description;

uint32_t climon2_client_get_interface(
    uint32_t requested_revision, const cm1_description **description);

#ifdef __cplusplus
}
#endif
#endif
