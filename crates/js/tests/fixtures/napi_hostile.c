/*
 * Hostile Node-API addon used by `tests/napi_sanitizers.rs`.
 *
 * It is deliberately not a well-behaved addon: every entry point passes data
 * that violates the N-API contract (invalid UTF-8, lengths that do not match
 * the buffer) so the host's validation can be exercised under ASan and by
 * ordinary regression tests. Compiled at test time; the host's napi_* symbols
 * are resolved through the executable's dynamic symbol table (`-rdynamic`).
 *
 * SPDX-License-Identifier: MIT
 * Copyright (c) 3va contributors
 */
#include <stddef.h>
#include <stdint.h>
#include <stdlib.h>

typedef void *napi_env;
typedef void *napi_value;
typedef void *napi_callback_info;
typedef int napi_status;
typedef napi_value (*napi_callback)(napi_env, napi_callback_info);

struct napi_property_descriptor {
  const char *utf8name;
  napi_value name;
  napi_value method;
  napi_value getter;
  napi_value setter;
  napi_value value;
  int attributes;
  void *data;
};

extern napi_status napi_create_string_utf8(napi_env, const char *, size_t,
                                           napi_value *);
extern napi_status napi_define_class(napi_env, const char *, size_t,
                                     napi_callback, void *, size_t,
                                     const struct napi_property_descriptor *,
                                     napi_value *);
extern napi_status napi_create_int32(napi_env, int32_t, napi_value *);
extern napi_status napi_set_named_property(napi_env, napi_value, const char *,
                                           napi_value);

static napi_value hostile_ctor(napi_env env, napi_callback_info info) {
  (void)env;
  (void)info;
  return 0;
}

/* Store `status` as a numeric export under `name`. */
static void record(napi_env env, napi_value exports, const char *name,
                   napi_status status) {
  napi_value v = 0;
  napi_create_int32(env, (int32_t)status, &v);
  napi_set_named_property(env, exports, name, v);
}

napi_value napi_register_module_v1(napi_env env, napi_value exports) {
  /* 0xff can never occur in well-formed UTF-8. */
  static const unsigned char bad[] = {0xff, 0xfe, 0x80, 0x41};

  napi_value out = 0;
  record(env, exports, "string_status",
         napi_create_string_utf8(env, (const char *)bad, sizeof(bad), &out));
  record(env, exports, "class_status",
         napi_define_class(env, (const char *)bad, sizeof(bad), hostile_ctor, 0,
                           0, 0, &out));

  /* ASan-only probe: claim a length far larger than the allocation. The host
   * cannot know the real buffer size, so this is an addon contract violation;
   * the probe exists to show what ASan reports. Gated so the normal suite
   * never hits it. */
  if (getenv("NAPI_HOSTILE_OOB") != NULL) {
    char *tiny = (char *)malloc(8);
    /* Valid UTF-8 for the 8 real bytes so the host's validator cannot bail
     * out before crossing into the ASan redzone. */
    for (int i = 0; i < 8; i++) {
      tiny[i] = 'A';
    }
    napi_value oob = 0;
    record(env, exports, "oob_status",
           napi_create_string_utf8(env, tiny, 8192, &oob));
  }
  return exports;
}
