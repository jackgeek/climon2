/*
 * Test-only stand-in for libclimon2_client.dylib, compiled by xtask's
 * release-check tests to prove each check rejects one defect. Never shipped.
 * RELEASE names the release text; EXTRA_EXPORT and HOOK_MARKER add defects.
 */
#include "client-v1.h"

static const uint8_t release[] = RELEASE;

static const cm1_description description = {
    .revision = CM1_REVISION,
    .release = {release, sizeof release - 1},
};

uint32_t climon2_client_get_interface(uint32_t revision,
                                      const cm1_description **out)
{
    if (out == 0)
        return CM1_INVALID_ARGUMENT;
    *out = revision == CM1_REVISION ? &description : 0;
    return *out ? CM1_SUCCESS : CM1_UNSUPPORTED_REVISION;
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
