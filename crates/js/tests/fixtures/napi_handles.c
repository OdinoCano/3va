/*
 * Well-behaved Node-API addon used by `tests/napi_handles.rs`.
 *
 * It creates handles in the three places Node-API releases them: a native
 * callback's implicit scope, an explicit handle scope, and an escapable scope.
 *
 * SPDX-License-Identifier: MIT
 * Copyright (c) 3va contributors
 */
#include <stddef.h>
#include <stdint.h>

typedef void *napi_env;
typedef void *napi_value;
typedef void *napi_callback_info;
typedef void *napi_handle_scope;
typedef void *napi_escapable_handle_scope;
typedef int napi_status;
typedef napi_value (*napi_callback)(napi_env, napi_callback_info);

extern napi_status napi_create_int32(napi_env, int32_t, napi_value *);
extern napi_status napi_create_string_utf8(napi_env, const char *, size_t,
                                           napi_value *);
extern napi_status napi_create_function(napi_env, const char *, size_t,
                                        napi_callback, void *, napi_value *);
extern napi_status napi_set_named_property(napi_env, napi_value, const char *,
                                           napi_value);
extern napi_status napi_get_cb_info(napi_env, napi_callback_info, size_t *,
                                    napi_value *, napi_value *, void **);
extern napi_status napi_get_value_int32(napi_env, napi_value, int32_t *);
extern napi_status napi_open_handle_scope(napi_env, napi_handle_scope *);
extern napi_status napi_close_handle_scope(napi_env, napi_handle_scope);
extern napi_status napi_open_escapable_handle_scope(
    napi_env, napi_escapable_handle_scope *);
extern napi_status napi_close_escapable_handle_scope(
    napi_env, napi_escapable_handle_scope);
extern napi_status napi_escape_handle(napi_env, napi_escapable_handle_scope,
                                      napi_value, napi_value *);

static int32_t arg_count(napi_env env, napi_callback_info info) {
  size_t argc = 1;
  napi_value argv[1] = {0};
  int32_t n = 0;
  napi_get_cb_info(env, info, &argc, argv, 0, 0);
  if (argc > 0) napi_get_value_int32(env, argv[0], &n);
  return n;
}

/* n handles in the callback's implicit scope, no explicit scope. */
static napi_value churn(napi_env env, napi_callback_info info) {
  int32_t n = arg_count(env, info);
  napi_value v = 0;
  for (int32_t i = 0; i < n; i++) napi_create_int32(env, i, &v);
  return 0;
}

/* n handles, each in its own explicit scope. Returns the live count seen
 * *inside* the last scope is not needed; the host test counts afterwards. */
static napi_value scoped(napi_env env, napi_callback_info info) {
  int32_t n = arg_count(env, info);
  for (int32_t i = 0; i < n; i++) {
    napi_handle_scope hs = 0;
    napi_value v = 0;
    napi_open_handle_scope(env, &hs);
    napi_create_int32(env, i, &v);
    napi_close_handle_scope(env, hs);
  }
  return 0;
}

/* A value created in an escapable scope must survive its close. */
static napi_value escape(napi_env env, napi_callback_info info) {
  (void)info;
  napi_escapable_handle_scope es = 0;
  napi_value inner = 0, outer = 0, junk = 0;
  napi_open_escapable_handle_scope(env, &es);
  napi_create_int32(env, 1, &junk);
  napi_create_string_utf8(env, "escaped", 7, &inner);
  napi_escape_handle(env, es, inner, &outer);
  napi_close_escapable_handle_scope(env, es);
  return outer;
}

static void export_fn(napi_env env, napi_value exports, const char *name,
                      napi_callback cb) {
  napi_value fn = 0;
  napi_create_function(env, name, 0, cb, 0, &fn);
  napi_set_named_property(env, exports, name, fn);
}

napi_value napi_register_module_v1(napi_env env, napi_value exports) {
  export_fn(env, exports, "churn", churn);
  export_fn(env, exports, "scoped", scoped);
  export_fn(env, exports, "escape", escape);
  return exports;
}
