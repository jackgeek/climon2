# Library compatibility check

**Status: agreed revision-1 specification, not implemented.**
The [monitor](../../../GLOSSARY.md#sessions) checks a loaded
[client library](../../../GLOSSARY.md#sessions) before creating an instance.
Use the [overview](../README.md), [glossary](../../../GLOSSARY.md) and
[terms](../TERMS.md).

## Schema and example

The frozen [C ABI](../TERMS.md#c-abi) bootstrap is declared in the
[header](client-v1.h). Request revision 1; success returns its immutable
description/table, exact release, capability tokens and qualified resource
requirements. Unsupported revision returns explicit status and null.

```c
const cm1_description *description = 0;
uint32_t status = climon2_client_get_interface(CM1_REVISION, &description);
if (status != CM1_SUCCESS)
    return (cm1_call){.code = status,
        .error = {CM1_ORIGIN_LIBRARY, 0,
            {(const uint8_t *)"bootstrap_failed", 16},
            {(const uint8_t *)"Interface lookup failed", 23}}};
return (cm1_call){0};
```

After lookup, compare exact release against immutable session metadata before
create, and reserve qualified resources. Revision compatibility never waives
release matching. Loading/lookup does not open a daemon connection or grant
control. Bootstrap metadata lives until process exit; rejected opened libraries
are retained too. Ordinary call diagnostics have a different, call-borrowed
lifetime. See [bootstrap rules](core-v1.md#representation-and-compatibility),
[startup](core-v1.md#startup-limits-and-resources), and
[embedding](../library-embedding.md).
This is not another daemon hello or serialized message.
[#65](https://github.com/jackgeek/climon2/issues/65) owns embedding;
[#71](https://github.com/jackgeek/climon2/issues/71) owns the declarations.
