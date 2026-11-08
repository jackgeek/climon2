# Response

**Status: agreed revision-1 specification, not implemented.**
The [client](../../../GLOSSARY.md#sessions) delivers a correlated
[response](../TERMS.md#response) to its caller. Use the [overview](../README.md),
[glossary](../../../GLOSSARY.md) and [terms](../TERMS.md).

## Schema and example

`CM1_RESPONSE` uses `cm1_record.data.response`: public request ID, outcome,
result tag, error and active result in the [header](client-v1.h).
The [complete result mapping](core-v1.md#requests-and-result-mapping) fixes
success, partial effects and known-final variants.

```c
cm1_record example = {0};
example.tag = CM1_RESPONSE;
example.generation = 1;
example.data.response.request_id = 7;
example.data.response.outcome = CM1_OK;
example.data.response.result_tag = CM1_ACKNOWLEDGEMENT;
example.data.response.result.changed = 0;
(void)example;
return (cm1_call){0};
```

Success has the operation's matching result and no error. Rejected guarantees no
execution; failed does not; indeterminate is neither success nor proof of no
effects. Non-success requires explicit origin/code/text and may preserve known
partial results. Unknown codes stay non-success; unknown/mismatched result tags
fail explicitly. There is no response for local admission rejection.

Responses share the causal event stream. Immediate daemon effects precede their
success; queue acceptance, Input forwarding, Focus state, geometry, Ready, Leave,
Kill daemon cleanup and local joins prove different things. See
[ordering](core-v1.md#events-and-ordering), [ownership](core-v1.md#function-calls-and-ownership)
and [wire Response](../daemon-client/response.md).
[#66](https://github.com/jackgeek/climon2/issues/66) owns correlation;
[#71](https://github.com/jackgeek/climon2/issues/71) owns native results.
