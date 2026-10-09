/*
 * Node-API addon used by `tests/napi_handles.rs` to exercise finalizers.
 *
 * `wrapOnce` attaches a finalizer to a fresh object that only exists inside the
 * native callback, so it is collectable as soon as the callback returns.
 * `wrapAndRemove` detaches it with `napi_remove_wrap`, and `doubleWrapStatus`
 * reports the status of a second `napi_wrap` on the same object.
 * `wrapKept` returns the object so the caller can keep it alive until the
 * environment is torn down. `churnFunctions` creates `n` functions with
 * `napi_create_function` and throws them away.
 *
 * SPDX-License-Identifier: MIT
 * Copyright (c) 3va contributors
 */
#include <stddef.h>
#include <stdint.h>

typedef void *napi_env;
typedef void *napi_value;
typedef void *napi_callback_info;
typedef int napi_status;
typedef napi_value (*napi_callback)(napi_env, napi_callback_info);
typedef void (*napi_finalize)(napi_env, void *, void *);

extern napi_status napi_create_int32(napi_env, int32_t, napi_value *);
extern napi_status napi_create_function(napi_env, const char *, size_t,
                                         napi_callback, void *, napi_value *);
extern napi_status napi_create_object(napi_env, napi_value *);
extern napi_status napi_set_named_property(napi_env, napi_value, const char *,
                                            napi_value);
extern napi_status napi_get_cb_info(napi_env, napi_callback_info, size_t *,
                                     napi_value *, napi_value *, void **);
extern napi_status napi_get_value_int32(napi_env, napi_value, int32_t *);
extern napi_status napi_wrap(napi_env, napi_value, void *, napi_finalize, void *,
                             napi_value *);
extern napi_status napi_remove_wrap(napi_env, napi_value, void **);
extern napi_status napi_set_instance_data(napi_env, void *, napi_finalize,
                                          void *);

static int g_wrap_finalized = 0;
static int g_instance_finalized = 0;

static void wrap_finalizer(napi_env env, void *data, void *hint) {
  (void)env;
  (void)data;
  (void)hint;
  g_wrap_finalized++;
}

static void instance_finalizer(napi_env env, void *data, void *hint) {
  (void)env;
  (void)data;
  (void)hint;
  g_instance_finalized++;
}

static int32_t arg_int(napi_env env, napi_callback_info info) {
  size_t argc = 1;
  napi_value argv[1] = {0};
  int32_t n = 0;
  napi_get_cb_info(env, info, &argc, argv, 0, 0);
  if (argc > 0) napi_get_value_int32(env, argv[0], &n);
  return n;
}

/* Wrap an object that goes out of scope with the callback. */
static napi_value wrap_once(napi_env env, napi_callback_info info) {
  (void)info;
  napi_value obj = 0;
  napi_create_object(env, &obj);
  napi_wrap(env, obj, (void *)0x1, wrap_finalizer, 0, 0);
  return 0;
}

/* Wrap then detach: the finalizer must not run. */
static napi_value wrap_and_remove(napi_env env, napi_callback_info info) {
  (void)info;
  napi_value obj = 0;
  void *out = 0;
  napi_create_object(env, &obj);
  napi_wrap(env, obj, (void *)0x2, wrap_finalizer, 0, 0);
  napi_remove_wrap(env, obj, &out);
  return 0;
}

/* A second napi_wrap on the same object must return napi_invalid_arg (1). */
static napi_value double_wrap_status(napi_env env, napi_callback_info info) {
  (void)info;
  napi_value obj = 0;
  napi_value result = 0;
  napi_create_object(env, &obj);
  napi_wrap(env, obj, (void *)0x3, wrap_finalizer, 0, 0);
  int32_t second = napi_wrap(env, obj, (void *)0x4, wrap_finalizer, 0, 0);
  napi_create_int32(env, second, &result);
  return result;
}

/* Return the wrapped object so the caller can keep it alive. */
static napi_value wrap_kept(napi_env env, napi_callback_info info) {
  (void)info;
  napi_value obj = 0;
  napi_create_object(env, &obj);
  napi_wrap(env, obj, (void *)0x5, wrap_finalizer, 0, 0);
  return obj;
}

static napi_value wrap_counter(napi_env env, napi_callback_info info) {
  (void)info;
  napi_value r = 0;
  napi_create_int32(env, g_wrap_finalized, &r);
  return r;
}

static napi_value set_instance(napi_env env, napi_callback_info info) {
  (void)info;
  napi_set_instance_data(env, (void *)0x6, instance_finalizer, 0);
  return 0;
}

static napi_value instance_counter(napi_env env, napi_callback_info info) {
  (void)info;
  napi_value r = 0;
  napi_create_int32(env, g_instance_finalized, &r);
  return r;
}

static napi_value churn_functions(napi_env env, napi_callback_info info) {
  int32_t n = arg_int(env, info);
  napi_value fn = 0;
  for (int32_t i = 0; i < n; i++) {
    napi_create_function(env, "f", 1, wrap_once, 0, &fn);
  }
  return 0;
}

static void export_fn(napi_env env, napi_value exports, const char *name,
                      napi_callback cb) {
  napi_value fn = 0;
  napi_create_function(env, name, 0, cb, 0, &fn);
  napi_set_named_property(env, exports, name, fn);
}

napi_value napi_register_module_v1(napi_env env, napi_value exports) {
  export_fn(env, exports, "wrapOnce", wrap_once);
  export_fn(env, exports, "wrapAndRemove", wrap_and_remove);
  export_fn(env, exports, "doubleWrapStatus", double_wrap_status);
  export_fn(env, exports, "wrapKept", wrap_kept);
  export_fn(env, exports, "wrapCounter", wrap_counter);
  export_fn(env, exports, "setInstance", set_instance);
  export_fn(env, exports, "instanceCounter", instance_counter);
  export_fn(env, exports, "churnFunctions", churn_functions);
  return exports;
}
