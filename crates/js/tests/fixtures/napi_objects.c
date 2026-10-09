/*
 * Node-API addon used by `tests/napi_objects.rs` for napi_get_prototype,
 * napi_has_own_property and napi_remove_wrap. Each function returns either the
 * result or, on a non-ok status, the negated status code as an int32.
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

extern napi_status napi_create_int32(napi_env, int32_t, napi_value *);
extern napi_status napi_get_boolean(napi_env, int, napi_value *);
extern napi_status napi_create_function(napi_env, const char *, size_t,
                                        napi_callback, void *, napi_value *);
extern napi_status napi_set_named_property(napi_env, napi_value, const char *,
                                           napi_value);
extern napi_status napi_get_cb_info(napi_env, napi_callback_info, size_t *,
                                    napi_value *, napi_value *, void **);
extern napi_status napi_get_prototype(napi_env, napi_value, napi_value *);
extern napi_status napi_has_own_property(napi_env, napi_value, napi_value,
                                         int *);
extern napi_status napi_wrap(napi_env, napi_value, void *, void *, void *,
                             napi_value *);
extern napi_status napi_remove_wrap(napi_env, napi_value, void **);
extern napi_status napi_get_typedarray_info(napi_env, napi_value, int *, size_t *,
                                            void **, napi_value *, size_t *);
extern napi_status napi_unwrap(napi_env, napi_value, void **);
extern napi_status napi_get_value_int32(napi_env, napi_value, int32_t *);
extern napi_status napi_create_string_utf8(napi_env, const char *, size_t,
                                           napi_value *);

/* Layout of Node-API's napi_property_descriptor (64 bytes). */
struct napi_property_descriptor {
  const char *utf8name;
  napi_value name;
  napi_callback method;
  napi_callback getter;
  napi_callback setter;
  napi_value value;
  int attributes;
  void *data;
};
extern napi_status napi_define_class(napi_env, const char *, size_t,
                                     napi_callback, void *, size_t,
                                     const struct napi_property_descriptor *,
                                     napi_value *);
enum { W = 1, E = 2, C = 4, S = 1 << 10 };

static napi_value status_value(napi_env env, napi_status st) {
  napi_value v = 0;
  napi_create_int32(env, -st, &v);
  return v;
}

static napi_value proto(napi_env env, napi_callback_info info) {
  size_t argc = 1;
  napi_value argv[1] = {0}, out = 0;
  napi_get_cb_info(env, info, &argc, argv, 0, 0);
  napi_status st = napi_get_prototype(env, argv[0], &out);
  return st == 0 ? out : status_value(env, st);
}

static napi_value own(napi_env env, napi_callback_info info) {
  size_t argc = 2;
  napi_value argv[2] = {0, 0}, out = 0;
  int has = 0;
  napi_get_cb_info(env, info, &argc, argv, 0, 0);
  napi_status st = napi_has_own_property(env, argv[0], argv[1], &has);
  if (st != 0) return status_value(env, st);
  napi_get_boolean(env, has, &out);
  return out;
}

/* Wrap, remove (must give back the same pointer), remove again (must fail).
 * Returns first_status*100 + second_status*10 + pointer_matches. */
static int magic = 7;
static napi_value wrap_remove(napi_env env, napi_callback_info info) {
  size_t argc = 1;
  napi_value argv[1] = {0}, out = 0;
  void *p = 0;
  napi_get_cb_info(env, info, &argc, argv, 0, 0);
  napi_wrap(env, argv[0], &magic, 0, 0, 0);
  napi_status first = napi_remove_wrap(env, argv[0], &p);
  int matches = (p == (void *)&magic);
  napi_status second = napi_remove_wrap(env, argv[0], &p);
  napi_create_int32(env, first * 100 + second * 10 + matches, &out);
  return out;
}

/* A class: constructor, method, accessor, static method and static value. */
static int32_t counters[8];
static int next_counter;

static int32_t *this_counter(napi_env env, napi_callback_info info) {
  size_t argc = 0;
  napi_value self = 0;
  void *p = 0;
  napi_get_cb_info(env, info, &argc, 0, &self, 0);
  napi_unwrap(env, self, &p);
  return (int32_t *)p;
}

static napi_value counter_ctor(napi_env env, napi_callback_info info) {
  size_t argc = 0;
  napi_value self = 0;
  napi_get_cb_info(env, info, &argc, 0, &self, 0);
  napi_wrap(env, self, &counters[next_counter++ % 8], 0, 0, 0);
  return self;
}

static napi_value counter_inc(napi_env env, napi_callback_info info) {
  napi_value out = 0;
  int32_t *p = this_counter(env, info);
  (*p)++;
  napi_create_int32(env, *p, &out);
  return out;
}

static napi_value counter_get(napi_env env, napi_callback_info info) {
  napi_value out = 0;
  napi_create_int32(env, *this_counter(env, info), &out);
  return out;
}

static napi_value counter_set(napi_env env, napi_callback_info info) {
  size_t argc = 1;
  napi_value argv[1] = {0};
  napi_get_cb_info(env, info, &argc, argv, 0, 0);
  napi_get_value_int32(env, argv[0], this_counter(env, info));
  return 0;
}

static napi_value counter_answer(napi_env env, napi_callback_info info) {
  (void)info;
  napi_value out = 0;
  napi_create_int32(env, 42, &out);
  return out;
}

/* napi_get_typedarray_info: returns type*10000 + length*100 + byte_offset, or the
 * negated status on error. */
static napi_value ta_info(napi_env env, napi_callback_info info) {
  size_t argc = 1;
  napi_value argv[1] = {0}, out = 0;
  int type = -1;
  size_t length = 0, offset = 0;
  napi_get_cb_info(env, info, &argc, argv, 0, 0);
  napi_status st =
      napi_get_typedarray_info(env, argv[0], &type, &length, 0, 0, &offset);
  if (st != 0) return status_value(env, st);
  napi_create_int32(env, type * 10000 + (int32_t)length * 100 + (int32_t)offset,
                    &out);
  return out;
}

static void export_fn(napi_env env, napi_value exports, const char *name,
                      napi_callback cb) {
  napi_value fn = 0;
  napi_create_function(env, name, 0, cb, 0, &fn);
  napi_set_named_property(env, exports, name, fn);
}

napi_value napi_register_module_v1(napi_env env, napi_value exports) {
  export_fn(env, exports, "proto", proto);
  export_fn(env, exports, "own", own);
  export_fn(env, exports, "wrapRemove", wrap_remove);
  export_fn(env, exports, "taInfo", ta_info);

  napi_value kind = 0, cls = 0;
  napi_create_string_utf8(env, "counter", 7, &kind);
  struct napi_property_descriptor props[] = {
      {"inc", 0, counter_inc, 0, 0, 0, W | C, 0},
      {"value", 0, 0, counter_get, counter_set, 0, E | C, 0},
      {"answer", 0, counter_answer, 0, 0, 0, S | W | C, 0},
      {"kind", 0, 0, 0, 0, kind, S | E, 0},
  };
  napi_define_class(env, "Counter", 7, counter_ctor, 0, 4, props, &cls);
  napi_set_named_property(env, exports, "Counter", cls);
  return exports;
}
