/*
 * Node-API addon with one slow async work item, used by `tests/napi_async.rs`.
 * `slow(ms)` returns a promise resolved with 42 from the completion callback after
 * the execute callback has slept `ms` milliseconds on a worker thread.
 *
 * SPDX-License-Identifier: MIT
 * Copyright (c) 3va contributors
 */
#include <stddef.h>
#include <stdint.h>
#include <unistd.h>

typedef void *napi_env;
typedef void *napi_value;
typedef void *napi_callback_info;
typedef void *napi_deferred;
typedef void *napi_async_work;
typedef int napi_status;
typedef napi_value (*napi_callback)(napi_env, napi_callback_info);

extern napi_status napi_create_int32(napi_env, int32_t, napi_value *);
extern napi_status napi_get_cb_info(napi_env, napi_callback_info, size_t *,
                                    napi_value *, napi_value *, void **);
extern napi_status napi_get_value_int32(napi_env, napi_value, int32_t *);
extern napi_status napi_create_string_utf8(napi_env, const char *, size_t,
                                           napi_value *);
extern napi_status napi_create_function(napi_env, const char *, size_t,
                                        napi_callback, void *, napi_value *);
extern napi_status napi_set_named_property(napi_env, napi_value, const char *,
                                           napi_value);
extern napi_status napi_create_promise(napi_env, napi_deferred *, napi_value *);
extern napi_status napi_resolve_deferred(napi_env, napi_deferred, napi_value);
extern napi_status napi_create_async_work(
    napi_env, napi_value, napi_value, void (*)(napi_env, void *),
    void (*)(napi_env, napi_status, void *), void *, napi_async_work *);
extern napi_status napi_queue_async_work(napi_env, napi_async_work);
extern napi_status napi_delete_async_work(napi_env, napi_async_work);

static napi_deferred deferred;
static napi_async_work work;
static int32_t delay_ms = 200;

static void execute(napi_env env, void *data) {
  (void)env;
  (void)data;
  usleep((useconds_t)delay_ms * 1000);
}

static void complete(napi_env env, napi_status status, void *data) {
  (void)status;
  (void)data;
  napi_value v = 0;
  napi_create_int32(env, 42, &v);
  napi_resolve_deferred(env, deferred, v);
  napi_delete_async_work(env, work);
}

static napi_value slow(napi_env env, napi_callback_info info) {
  size_t argc = 1;
  napi_value argv[1] = {0}, promise = 0, name = 0;
  napi_get_cb_info(env, info, &argc, argv, 0, 0);
  if (argc > 0) napi_get_value_int32(env, argv[0], &delay_ms);
  napi_create_promise(env, &deferred, &promise);
  napi_create_string_utf8(env, "slow", 4, &name);
  napi_create_async_work(env, 0, name, execute, complete, 0, &work);
  napi_queue_async_work(env, work);
  return promise;
}

napi_value napi_register_module_v1(napi_env env, napi_value exports) {
  napi_value fn = 0;
  napi_create_function(env, "slow", 4, slow, 0, &fn);
  napi_set_named_property(env, exports, "slow", fn);
  return exports;
}
