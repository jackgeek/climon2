# Create local instance

**Status: agreed revision-1 specification. The
[client library](../../../GLOSSARY.md#sessions) implements create
([#85](https://github.com/jackgeek/climon2/issues/85)), copying the session ID
that Open checks the metadata and hello against
([#93](https://github.com/jackgeek/climon2/issues/93)); its caller is not
implemented.**
The [monitor](../../../GLOSSARY.md#sessions) creates a
[client](../../../GLOSSARY.md#sessions) on its dedicated owner thread.
Use the [overview](../README.md), [glossary](../../../GLOSSARY.md) and
[terms](../TERMS.md).

## Schema and example

`create(const cm1_startup *, cm1_created *)` returns `cm1_call`.
Startup contains session [GUID](../TERMS.md#guid), fresh
[generation](../TERMS.md#connection-generation), exact release, absolute [native](../TERMS.md#native-path)
metadata/home/runtime paths, validated key settings, explicit diagnostics and
fully resolved admitted limits. Success returns instance/communication/control
[handles](../TERMS.md#opaque-handle) and equal effective limits. See [header](client-v1.h),
[complete startup fields](core-v1.md#startup-limits-and-resources) and
[`example_create`](examples.c), which supplies every required field.

```c
cm1_startup startup = {0};
static const uint8_t release[] = "0.1.0";
static const uint8_t metadata[] = "/tmp/home/sessions/session.json";
static const uint8_t home[] = "/tmp/home";
static const uint8_t runtime[] = "/tmp/runtime";
static const uint8_t filter[] = "info";
startup.session_id = (cm1_guid){{0x00, 0x11, 0x22, 0x33, 0x44, 0x55,
    0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff}};
startup.generation = generation;
startup.expected_release = (cm1_bytes){release, sizeof release - 1};
startup.metadata_path = (cm1_path){CM1_NATIVE_UNIX, 0,
    {metadata, sizeof metadata - 1}};
startup.home_path = (cm1_path){CM1_NATIVE_UNIX, 0, {home, sizeof home - 1}};
startup.runtime_path = (cm1_path){CM1_NATIVE_UNIX, 0,
    {runtime, sizeof runtime - 1}};
startup.diagnostics.filter = (cm1_bytes){filter, sizeof filter - 1};
extern cm1_limits example_limits(void);
startup.limits = example_limits();
cm1_created created = {0};
return api->create(&startup, &created);
```

This disables both client keys and file logging explicitly; `example_limits`
uses resolved illustrative values after reserving monitor model capacity.
The paths/release/session must match real validated installation/metadata before
Open; examples do not create those files or guarantee admission.

Bootstrap exact identity/revision and qualified resources precede create.
Copy nested buffers before return. No daemon [I/O](../TERMS.md#io)/workers;
partial local setup
rolls back, failure returns no handles. Unsupported limits/settings reject
explicitly, never fall back to environment/defaults. Caller owns host streams,
signals/cwd/configuration. See [ownership/errors](core-v1.md#function-calls-and-ownership),
[Open](request-open.md), and [embedding](../library-embedding.md).
[#65](https://github.com/jackgeek/climon2/issues/65) owns lifetime;
[#71](https://github.com/jackgeek/climon2/issues/71) owns startup declarations.
