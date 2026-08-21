#ifndef FOD_LIBFOD_H
#define FOD_LIBFOD_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

const char *fod_version_label(void);
const char *fod_last_error_message(void);

size_t fod_program_count(void);
const char *fod_program_name(size_t index);
const char *fod_program_binary(size_t index);
const char *fod_program_description(size_t index);
intptr_t fod_program_find(const char *name);

int fod_program_run(const char *name, int argc, const char *const *argv);
int fod_bootstrap(int argc, const char *const *argv);
int fod_mkfs(int argc, const char *const *argv);
int fod_config(int argc, const char *const *argv);
int fod_change(int argc, const char *const *argv);
int fod_indexer(int argc, const char *const *argv);
int fod_monitor(int argc, const char *const *argv);
int fod_rust_fuse(int argc, const char *const *argv);
int fod_mount(int argc, const char *const *argv);

#ifdef __cplusplus
}
#endif

#endif
