//******************************************************************************
// Copyright (c) 2018, The Regents of the University of California (Regents).
// All Rights Reserved. See LICENSE for license details.
//------------------------------------------------------------------------------
#ifndef _ENCLAVE_H_
#define _ENCLAVE_H_

#ifndef TARGET_PLATFORM_HEADER
#error "SM requires a defined platform to build"
#endif

#include "sm.h"
#include "bits.h"
#include "vm.h"
#include "pmp.h"
#include "thread.h"
#include "crypto.h"

// Special target platform header, set by configure script
#include TARGET_PLATFORM_HEADER

#define ATTEST_DATA_MAXLEN  1024
#define ENCLAVE_REGIONS_MAX 8
/* TODO: does not support multithreaded enclave yet */
#define MAX_ENCL_THREADS 1

typedef enum {
  DESTROYED = -2,
  INVALID = -1,
  FRESH = 0,
  INITIALIZED,
  RUNNING,
  ALLOCATED,
} enclave_state;

/* Enclave stop reasons requested */
#define STOP_TIMER_INTERRUPT  0
#define STOP_EDGE_CALL_HOST   1
#define STOP_EXIT_ENCLAVE     2

/* For now, eid's are a simple unsigned int */
typedef unsigned int enclave_id;

/* Metadata around memory regions associate with this enclave
 * EPM is the 'home' for the enclave, contains runtime code/etc
 * UTM is the untrusted shared pages
 * OTHER is managed by some other component (e.g. platform_)
 * INVALID is an unused index
 */
enum enclave_region_type{
  REGION_INVALID,
  REGION_EPM,
  REGION_UTM,
  REGION_OTHER,
};

struct enclave_region
{
  region_id pmp_rid;
  enum enclave_region_type type;
};

/* enclave metadata */
struct enclave
{
  //spinlock_t lock; //local enclave lock. we don't need this until we have multithreaded enclave
  enclave_id eid; //enclave id
  unsigned long encl_satp; // enclave's page table base
  enclave_state state; // global state of the enclave

  /* Physical memory regions associate with this enclave */
  struct enclave_region regions[ENCLAVE_REGIONS_MAX];

  /* measurement */
  byte hash[MDSIZE];
  byte sign[SIGNATURE_SIZE];

  /* parameters */
  struct runtime_va_params_t params;
  struct runtime_pa_params pa_params;

  /* enclave execution context */
  unsigned int n_thread;
  struct thread_state threads[MAX_ENCL_THREADS];

  struct platform_enclave_data ped;
};

/* attestation reports */
struct enclave_report
{
  byte hash[MDSIZE];
  uint64_t data_len;
  byte data[ATTEST_DATA_MAXLEN];
  byte signature[SIGNATURE_SIZE];
};
struct sm_report
{
  byte hash[MDSIZE];
  byte public_key[PUBLIC_KEY_SIZE];
  byte signature[SIGNATURE_SIZE];
};
struct report
{
  struct enclave_report enclave;
  struct sm_report sm;
  byte dev_public_key[PUBLIC_KEY_SIZE];
};

/*** SBI functions & external functions ***/
// callables from the host
enclave_ret_code create_enclave(struct keystone_sbi_create create_args);
enclave_ret_code destroy_enclave(enclave_id eid);
enclave_ret_code run_enclave(uintptr_t* host_regs, enclave_id eid);
enclave_ret_code resume_enclave(uintptr_t* regs, enclave_id eid);
// callables from the enclave
enclave_ret_code exit_enclave(uintptr_t* regs, unsigned long retval, enclave_id eid);
enclave_ret_code stop_enclave(uintptr_t* regs, uint64_t request, enclave_id eid);
enclave_ret_code attest_enclave(uintptr_t report, uintptr_t data, uintptr_t size, enclave_id eid);
/* attestation and virtual mapping validation */
enclave_ret_code validate_and_hash_enclave(struct enclave* enclave);
// TODO: These functions are supposed to be internal functions.
void enclave_init_metadata();
enclave_ret_code copy_from_host(void* source, void* dest, size_t size);
int get_enclave_region_index(enclave_id eid, enum enclave_region_type type);
uintptr_t get_enclave_region_base(enclave_id eid, int memid);
uintptr_t get_enclave_region_size(enclave_id eid, int memid);

// Here follow definitions for Rust interop; don't use these directly.
#define ENCL_MAX  16
extern struct enclave enclaves[ENCL_MAX];

enclave_ret_code context_switch_to_enclave(uintptr_t* regs,
                                           enclave_id eid,
                                           int load_parameters);
void context_switch_to_host(uintptr_t* encl_regs,
                            enclave_id eid);
enclave_ret_code copy_from_enclave(struct enclave* enclave,
                                   void* dest, void* source, size_t size);
enclave_ret_code copy_to_enclave(struct enclave* enclave,
                                 void* dest, void* source, size_t size);
void enclave_lock(void);
void enclave_unlock(void);

#endif
