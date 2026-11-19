#include "client-v1.h"
#include <stddef.h>

_Static_assert(sizeof(void *) == 8, "revision 1 requires a 64-bit target");
_Static_assert(sizeof(cm1_guid) == 16, "canonical GUID bytes");
_Static_assert(sizeof(cm1_utc) == 24, "millisecond UTC without NUL");
_Static_assert(sizeof(cm1_bytes) == 16, "pointer followed by uint64 length");
_Static_assert(offsetof(cm1_bytes, length) == 8, "buffer field order");
_Static_assert(sizeof(cm1_authority) == 24, "three independent counters");
_Static_assert(offsetof(cm1_request, request_id) == 8, "request prefix");
_Static_assert(offsetof(cm1_request, args) == 16, "request union alignment");
_Static_assert(offsetof(cm1_record, generation) == 8, "delivery prefix");
_Static_assert(offsetof(cm1_record, data) == 16, "delivery union alignment");
_Static_assert(offsetof(cm1_interface, create) == 0, "function table prefix");
_Static_assert(sizeof(cm1_interface) == 11 * 8, "frozen function table");
_Static_assert(CM1_OPEN == 1 && CM1_FOCUS == 10, "native request tags");
_Static_assert(CM1_RESPONSE == 1 && CM1_FINAL == 12, "native delivery tags");
_Static_assert(CM1_OK == 1 && CM1_INDETERMINATE == 4, "outcome tags");
