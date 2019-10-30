# Keystone Enclave Security Monitor

## Overview

These relevant source files in the `sm` directory appear in rough order of decreasing abstraction.

- `sm.rs`: Handles initialization routines for the entire security monitor, particularly with regards to initial setup of Physical Memory Protection (PMP) regions.
- `sm-sbi.c`: Implements the Secure Binary Interface (SBI) system-call front-end, through which user programs may interface with the security monitor.
- `enclave.rs`:  Handles the bulk of the system-call back-end, and provides functionality for enclave management, data transfer between normal and machine mode, and context switching.
- `platform/<PLATFORM>/<PLATFORM>.c`: Provides board-specific routines relevant for different syscall implementations.
- `pmp.c`, `pmp.rs`: Provides driver for allocating and handling PMP regions, so that enclaves and the security monitor are adequately isolated from the rest of the system.
- `crypto.rs`, `sha3/*`, `ed25519/*`: Provide cryptographic hashing and public key authentication routines for attestation reports.

## List of system calls

All the following system calls return an `enclave_ret_code` enumeration.

- `size_t sm_create_enclave(keystone_sbi_create *create_args)`
  Initialize a new enclave with the provided memory regions and miscellaneous parameters.
 
- `size_t sm_destroy_enclave(size_t eid)`
  Deinitializes all data associated with the given enclave ID. After calling this function, the input EID will no longer be valid.
  
- `size_t sm_run_enclave(uintptr_t (*regs)[32], unsigned long eid)`
  Schedules the given enclave to run for the first time, swapping out the given host registers.

- `size_t sm_exit_enclave(uintptr_t (*regs)[32], unsigned long retval)`
  Deschedules the currently running enclave, returning to the host process.

- `size_t sm_stop_enclave(uintptr_t (*regs)[32], unsigned long request)`
  Interrupts the currently running enclave due to a trap or machine interrupt.

- `size_t sm_resume_enclave(uintptr_t (*regs)[32], size_t eid)`
  Reschedules the given enclave after having stopped its execution.
  
- `size_t sm_attest_enclave(uintptr_t report, uintptr_t data, size_t size)`
  Generates a signature of the given enclave data region and incorporates it into an attestation report.

- `size_t sm_random(void)`
  Generates a random number.

- `size_t sm_call_plugin(size_t plugin_id, size_t call_id, size_t arg0, size_t arg1)`

## Back-end internals

### Enclave

#### Allocation and Deinitialization

Enclaves are allocated in an array of `N` `Enclave` structs, where free enclaves are determined by atomically scanning through a bitset of reserved EIDs. If no EID is free, `create_enclave` will return `ENCLAVE_NO_FREE_RESOURCE`. The resulting `Eid` struct will free its bit in the bitset upon deinitialization, which happens at the deinitialization of the `Enclave` struct through RAII.

The enclave also contains up to a fixed amount of `PmpRegion`s in a map of PMP region types and the `PmpRegion` member. These members are also managed through RAII, and so are freed at `Enclave` deinitialization.

All accesses to `Enclave`s are wrapped in a lock over the entire array of `N` `Enclave`s. Because of Rust's borrowing rules, this means that no two `Enclave`s may be modified concurrently with the current implementation, even with two different enclaves executing on two different CPUs.

#### Context switching

All context switch operations (run, exit, stop, resume) are only applied upon returning from the SBI call. They are triggered by setting CSR registers and updating the PMP regions within the corresponding system calls.
