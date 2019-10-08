use core::mem::{forget, size_of, zeroed};
use core::slice;

use util::ctypes::*;
use util::insert_field;
use crate::bindings::*;
use crate::sm;

const SATP_MODE_CHOICE: usize = insert_field!(0, SATP64_MODE as usize, SATP_MODE_SV39 as usize);

fn enclave_exists(eid: enclave_id) -> bool {
  unsafe {
    enclaves[eid as usize].state >= enclave_state_FRESH
  }
}



/* Ensures that dest ptr is in host, not in enclave regions
 */
#[no_mangle]
pub unsafe extern fn copy_word_to_host(dest_ptr: *mut usize, value: usize) -> enclave_ret_code 
{
  enclave_lock();
  let region_overlap = pmp_detect_region_overlap_atomic(dest_ptr as usize,
                                                        size_of::<usize>());
  if region_overlap == 0 {
    *dest_ptr = value;
  }
  enclave_unlock();

  if region_overlap != 0 {
    ENCLAVE_REGION_OVERLAPS as enclave_ret_code
  } else {
    ENCLAVE_SUCCESS as enclave_ret_code
  }
}

// TODO: This function is externally used by sm-sbi.c.
// Change it to be internal (remove from the enclave.h and make static)
/* Internal function enforcing a copy source is from the untrusted world.
 * Does NOT do verification of dest, assumes caller knows what that is.
 * Dest should be inside the SM memory.
 */
#[no_mangle]
pub unsafe extern fn copy_from_host(source: *mut c_void, dest: *mut c_void, size: usize) -> enclave_ret_code
{
  enclave_lock();
  let region_overlap = pmp_detect_region_overlap_atomic(source as usize, size);
  // TODO: Validate that dest is inside the SM.
  if region_overlap == 0 {
    dest.copy_from_nonoverlapping(source, size);
  }
  enclave_unlock();

  if region_overlap != 0 {
    ENCLAVE_REGION_OVERLAPS as enclave_ret_code
  } else {
    ENCLAVE_SUCCESS as enclave_ret_code
  }
}



#[no_mangle]
pub extern fn get_enclave_region_index(eid: enclave_id, ty: enclave_region_type) -> c_int
{
  let eid = eid as usize;

  for i in 0..ENCLAVE_REGIONS_MAX {
    if unsafe { enclaves[eid] }.regions[i as usize].type_ == ty {
      return i as c_int;
    }
  }
  // No such region for this enclave
  -1
}

#[no_mangle]
pub extern fn get_enclave_region_size(eid: enclave_id, memid: c_int) -> usize
{
  let eid = eid as usize;

  if 0 <= memid && memid < ENCLAVE_REGIONS_MAX as c_int {
    let size = unsafe {
        pmp_region_get_size(enclaves[eid].regions[memid as usize].pmp_rid)
    };
    // TODO: u64<->usize mismatch
    return size as usize;
  }

  0
}

#[no_mangle]
pub unsafe extern fn get_enclave_region_base(eid: enclave_id, memid: c_int) -> usize
{
  let eid = eid as usize;

  if 0 <= memid && memid < ENCLAVE_REGIONS_MAX as c_int {
    let addr = pmp_region_get_addr(enclaves[eid].regions[memid as usize].pmp_rid);
    // TODO: u64<->usize mismatch
    return addr as usize;
  }

  0
}



#[no_mangle]
pub extern fn attest_enclave(report_ptr: usize, data: usize, size: usize, eid: enclave_id) -> enclave_ret_code 
{
  let eid = eid as usize;

  let mut report = report {
    dev_public_key: [0u8; PUBLIC_KEY_SIZE as usize],
    enclave: enclave_report {
      data: [0u8; 1024],
      data_len: 0,
      hash: [0u8; MDSIZE as usize],
      signature: [0u8; SIGNATURE_SIZE as usize],
    },
    sm: sm_report {
      hash: [0u8; MDSIZE as usize],
      public_key: [0u8; PUBLIC_KEY_SIZE as usize],
      signature: [0u8; SIGNATURE_SIZE as usize],
    },
  };

  if size > ATTEST_DATA_MAXLEN as usize {
    return ENCLAVE_ILLEGAL_ARGUMENT as enclave_ret_code;
  }

  let attestable = unsafe {
    enclave_lock();
    let attestable = enclaves[eid].state >= enclave_state_INITIALIZED;
    enclave_unlock();
    attestable
  };

  if !attestable {
    return ENCLAVE_NOT_INITIALIZED as enclave_ret_code;
  }

  /* copy data to be signed */
  let dst_data_ptr = report.enclave.data.as_mut_ptr() as *mut c_void;
  let src_data_ptr = data as *mut c_void;

  let ret = unsafe {
    copy_from_enclave(&mut enclaves[eid],
      dst_data_ptr,
      src_data_ptr,
      size)
  };
  report.enclave.data_len = size as u64;

  if ret != 0 {
    return ret;
  }

  unsafe {
    report.dev_public_key = sm::dev_public_key;
    report.sm.hash = sm::sm_hash;
    report.sm.public_key = sm::sm_public_key;
    report.sm.signature = sm::sm_signature;
    report.enclave.hash = enclaves[eid].hash;
    //memcpy(report.enclave.hash, enclaves[eid].hash, MDSIZE);
  }

  unsafe {
    let enclave = &report.enclave as *const enclave_report as *const u8;
    let enclave_slice = slice::from_raw_parts(enclave, size_of::<enclave_report>());
    let enclave_slice = &enclave_slice[..enclave_slice.len() - SIGNATURE_SIZE as usize];
    let enclave_slice = &enclave_slice[..enclave_slice.len() - (ATTEST_DATA_MAXLEN as usize) + size];

    sm::sm_sign(&mut report.enclave.signature, enclave_slice);
  }

  /* copy report to the enclave */
  let dst_report_ptr = report_ptr as *mut c_void;
  let src_report_ptr = &mut report as *mut report as *mut c_void;

  let ret = unsafe {
    copy_to_enclave(&mut enclaves[eid],
      dst_report_ptr,
      src_report_ptr,
      size_of::<report>())
  };

  if ret != 0 {
    return ret;
  }

  return ENCLAVE_SUCCESS as enclave_ret_code;
}



struct PmpRegion {
    region: c_int 
}

impl PmpRegion {
    fn reserve(base: usize, size: usize, prio: pmp_priority) -> Result<Self, c_int> {
        let region = unsafe {
            let mut region = 0;
            let err = pmp_region_init_atomic(base, size as u64, prio, &mut region, 0);
            if err != 0 {
                return Err(err);
            }
            region
        };

        Ok(Self {
            region
        })
    }

    fn leak(self) -> c_int {
        let out = self.region;
        forget(self);
        out
    }

    fn set_global(&mut self, prop: u8) -> Result<(), c_int> {
        let err = unsafe {
            pmp_set_global(self.region, prop)
        };
        if err == 0 { Ok(()) }
        else { Err(err) }
    }
}

impl Drop for PmpRegion {
    fn drop(&mut self) {
      unsafe {
          pmp_region_free_atomic(self.region);
      }
    }
}

struct Eid {
    eid: enclave_id
}

impl Eid {
    fn reserve() -> Result<Eid, usize> {
        let mut eid = 0;
        let err = unsafe {
            encl_alloc_eid(&mut eid)
        };
        if err != 0 {
            return Err(err);
        }
        Ok(Self {
            eid
        })
    }

    fn leak(self) -> enclave_id {
        let out = self.eid;
        forget(self);
        out
    }
}

impl Drop for Eid {
    fn drop(&mut self) {
        unsafe {
            encl_free_eid(self.eid);
        }
    }
}

/* This handles creation of a new enclave, based on arguments provided
 * by the untrusted host.
 *
 * This may fail if: it cannot allocate PMP regions, EIDs, etc
 */
#[no_mangle]
pub unsafe extern fn create_enclave(create_args: keystone_sbi_create) -> enclave_ret_code
{
  /* EPM and UTM parameters */
  let base = create_args.epm_region.paddr;
  let size = create_args.epm_region.size;
  let utbase = create_args.utm_region.paddr;
  let utsize = create_args.utm_region.size;
  let eidptr = create_args.eid_pptr as *mut usize;

  /* Runtime parameters */
  if is_create_args_valid(&create_args) == 0 {
    return ENCLAVE_ILLEGAL_ARGUMENT as enclave_ret_code;
  }

  /* set va params */
  let params = create_args.params;
  let pa_params = runtime_pa_params {
    dram_base: base,
    dram_size: size,
    runtime_base: create_args.runtime_paddr,
    user_base: create_args.user_paddr,
    free_base: create_args.free_paddr,
  };


  // allocate eid
  let eid_reservation = Eid::reserve();
  let eid_reservation = if let Ok(e) = eid_reservation { e }
                        else { return ENCLAVE_NO_FREE_RESOURCE as enclave_ret_code };
  let eid = eid_reservation.eid as usize;

  // create a PMP region bound to the enclave
  let ret = ENCLAVE_PMP_FAILURE as enclave_ret_code;
  let region = PmpRegion::reserve(base, size, pmp_priority_PMP_PRI_ANY);
  let mut region = if let Ok(r) = region { r }
                   else { return ret };

  // create PMP region for shared memory
  let shared_region = PmpRegion::reserve(utbase, utsize, pmp_priority_PMP_PRI_BOTTOM);
  let shared_region = if let Ok(r) = shared_region { r }
                      else { return ret };

  // set pmp registers for private region (not shared)
  if let Err(_) = region.set_global(PMP_NO_PERM as u8) {
    return ret
  }

  // cleanup some memory regions for sanity See issue #38
  clean_enclave_memory(utbase, utsize);

  // initialize enclave metadata
  let mut enc = enclave {
      eid: eid as u32,

      regions: [
        enclave_region {
          pmp_rid: zeroed(),
          type_: 0
        };
        ENCLAVE_REGIONS_MAX as usize
      ],

      hash: zeroed(),
      ped: zeroed(),
      threads: zeroed(),
      sign: zeroed(),
      state: enclave_state_FRESH,

      encl_satp: ((base >> RISCV_PGSHIFT) | SATP_MODE_CHOICE),
      n_thread: 0,
      params: params,
      pa_params: pa_params,
  };

  enc.regions[0].pmp_rid = region.leak();
  enc.regions[0].type_ = enclave_region_type_REGION_EPM;
  enc.regions[1].pmp_rid = shared_region.leak();
  enc.regions[1].type_ = enclave_region_type_REGION_UTM;

  /* Init enclave state (regs etc) */
  clean_state(&mut enc.threads[0]);

  unsafe {
    enclaves[eid] = enc;
  }

  /* Platform create happens as the last thing before hashing/etc since
     it may modify the enclave struct */
  let ret = platform_create_enclave(&mut enclaves[eid]);
  if ret != ENCLAVE_SUCCESS as usize {
    return ret;
  }

  /* Validate memory, prepare hash and signature for attestation */
  enclave_lock();
  enclaves[eid].state = enclave_state_FRESH;
  let ret = validate_and_hash_enclave(&mut enclaves[eid]);
  enclave_unlock();

  if ret != ENCLAVE_SUCCESS as usize {
      platform_destroy_enclave(&mut enclaves[eid]);
  }

  /* EIDs are unsigned int in size, copy via simple copy */
  copy_word_to_host(eidptr, eid);

  eid_reservation.leak();
  return ENCLAVE_SUCCESS as enclave_ret_code;
}



#[no_mangle]
pub extern fn run_enclave(host_regs: *mut usize, eid: enclave_id) -> enclave_ret_code 
{
  let runnable = unsafe {
      enclave_lock();
      
      enclave_exists(eid)
          && enclaves[eid as usize].n_thread < MAX_ENCL_THREADS
  };

  unsafe {
      if runnable {
        enclaves[eid as usize].state = enclave_state_RUNNING;
        enclaves[eid as usize].n_thread += 1;
      }
      enclave_unlock();
  }

  if !runnable {
    return ENCLAVE_NOT_RUNNABLE as enclave_ret_code;
  }

  // Enclave is OK to run, context switch to it
  unsafe {
      context_switch_to_enclave(host_regs, eid as u32, 1)
  }
}

#[no_mangle]
pub extern fn exit_enclave(encl_regs: *mut usize, retval: c_ulong, eid: enclave_id) -> enclave_ret_code
{
  let eid = eid as usize;

  let exitable = unsafe {
    enclave_lock();
    let out = enclaves[eid].state == enclave_state_RUNNING;
    enclave_unlock();
    out
  };

  if !exitable {
    return ENCLAVE_NOT_RUNNING as enclave_ret_code;
  }

  unsafe {
    context_switch_to_host(encl_regs, eid as u32);
  }

  // update enclave state
  unsafe {
      enclave_lock();
      enclaves[eid].n_thread -= 1;
      if enclaves[eid].n_thread == 0 {
        enclaves[eid].state = enclave_state_INITIALIZED;
      }
      enclave_unlock();
  }

  return ENCLAVE_SUCCESS as enclave_ret_code;
}

#[no_mangle]
pub extern fn stop_enclave(encl_regs: *mut usize, request: u64, eid: enclave_id) -> enclave_ret_code
{
  let eid = eid as usize;

  let stoppable = unsafe {
      enclave_lock();
      let out = enclaves[eid].state == enclave_state_RUNNING;
      enclave_unlock();
      out
  };

  if !stoppable {
    return ENCLAVE_NOT_RUNNING as enclave_ret_code;
  }

  unsafe {
    context_switch_to_host(encl_regs, eid as u32);
  }

  match request {
      n if n == STOP_TIMER_INTERRUPT as u64 =>
          ENCLAVE_INTERRUPTED as enclave_ret_code,
      n if n == STOP_EDGE_CALL_HOST as u64 =>
          ENCLAVE_EDGE_CALL_HOST as enclave_ret_code,
      _ =>
          ENCLAVE_UNKNOWN_ERROR as enclave_ret_code
  }
}

#[no_mangle]
pub extern fn resume_enclave(host_regs: *mut usize, eid: enclave_id) -> enclave_ret_code
{
  let eid = eid as usize;

  let resumable = unsafe {
      enclave_lock();
      let out = enclaves[eid].state == enclave_state_RUNNING // not necessary?
               && enclaves[eid].n_thread > 0; // not necessary
      enclave_unlock();
      out
  };

  if !resumable {
    return ENCLAVE_NOT_RESUMABLE as enclave_ret_code;
  }

  // Enclave is OK to resume, context switch to it
  return unsafe {
      context_switch_to_enclave(host_regs, eid as u32, 0)
  }
}

