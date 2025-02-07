use std::io::{Read, Write};
use std::fmt::{self, Display, Debug};
use std::mem::{self, MaybeUninit};
use std::os::raw::{c_void, c_uint, c_int};
use std::ptr;

use fdk_aac_sys as sys;

pub use sys::AACENC_InfoStruct as InfoStruct;

pub enum EncoderError {
    Io(std::io::Error),
    FdkAac(sys::AACENC_ERROR),
}

impl EncoderError {
    fn message(&self) -> &'static str {
        match self {
            EncoderError::FdkAac(sys::AACENC_ERROR_AACENC_INVALID_HANDLE) => "Handle passed to function call was invalid.",
            EncoderError::FdkAac(sys::AACENC_ERROR_AACENC_MEMORY_ERROR) => "Memory allocation failed.",
            EncoderError::FdkAac(sys::AACENC_ERROR_AACENC_UNSUPPORTED_PARAMETER) => "Parameter not available.",
            EncoderError::FdkAac(sys::AACENC_ERROR_AACENC_INVALID_CONFIG) => "Configuration not provided.",
            EncoderError::FdkAac(sys::AACENC_ERROR_AACENC_INIT_ERROR) => "General initialization error.",
            EncoderError::FdkAac(sys::AACENC_ERROR_AACENC_INIT_AAC_ERROR) => "AAC library initialization error.",
            EncoderError::FdkAac(sys::AACENC_ERROR_AACENC_INIT_SBR_ERROR) => "SBR library initialization error.",
            EncoderError::FdkAac(sys::AACENC_ERROR_AACENC_INIT_TP_ERROR) => "Transport library initialization error.",
            EncoderError::FdkAac(sys::AACENC_ERROR_AACENC_INIT_META_ERROR) => "Meta data library initialization error.",
            EncoderError::FdkAac(sys::AACENC_ERROR_AACENC_INIT_MPS_ERROR) => "MPS library initialization error.",
            EncoderError::FdkAac(sys::AACENC_ERROR_AACENC_ENCODE_ERROR) => "The encoding process was interrupted by an unexpected error.",
            EncoderError::FdkAac(_) => "Unknown error",
            EncoderError::Io(_e) => "io error",
        }
    }

    fn code(&self) -> u32 {
        match self {
            EncoderError::FdkAac(code) => *code,
            EncoderError::Io(_e) => 0,
        }
    }
}

impl std::error::Error for EncoderError {
}

impl Debug for EncoderError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "EncoderError {{ code: {:?}, message: {:?} }}", self.code(), self.message())
    }
}

impl Display for EncoderError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.message())
    }
}

impl From<std::io::Error> for EncoderError {
    fn from(err: std::io::Error) -> Self {
        EncoderError::Io(err)
    }
}

fn check(e: sys::AACENC_ERROR) -> Result<(), EncoderError> {
    if e == sys::AACENC_ERROR_AACENC_OK {
        Ok(())
    } else {
        Err(EncoderError::FdkAac(e))
    }
}

struct EncoderHandle {
    ptr: sys::HANDLE_AACENCODER,
}

impl EncoderHandle {
    pub fn alloc(max_modules: usize, max_channels: usize) -> Result<Self, EncoderError> {
        let mut ptr: sys::HANDLE_AACENCODER = ptr::null_mut();
        check(unsafe {
            sys::aacEncOpen(&mut ptr as *mut _, max_modules as c_uint, max_channels as c_uint)
        })?;
        Ok(EncoderHandle { ptr })
    }
}

impl Drop for EncoderHandle {
    fn drop(&mut self) {
        unsafe { sys::aacEncClose(&mut self.ptr as *mut _); }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum BitRate {
    Cbr(u32),
    VbrVeryLow,
    VbrLow,
    VbrMedium,
    VbrHigh,
    VbrVeryHigh,
}

pub struct EncoderParams {
    pub bit_rate: BitRate,
    pub sample_rate: u32,
    pub transport: Transport,
}

pub struct Encoder {
    handle: EncoderHandle,
}

#[derive(Debug)]
pub enum Transport {
    Adts,
    Raw,
}

#[derive(Debug)]
pub struct EncodeInfo {
    pub input_consumed: usize,
    pub output_size: usize,
}

impl Encoder {
    pub fn new(params: EncoderParams) -> Result<Self, EncoderError> {
        let handle = EncoderHandle::alloc(0, 2 /* hardcode stereo */)?;

        unsafe {
            // hardcode MPEG-4 AAC Low Complexity for now:
            check(sys::aacEncoder_SetParam(handle.ptr, sys::AACENC_PARAM_AACENC_AOT, 2))?;

            let bitrate_mode = match params.bit_rate {
                BitRate::Cbr(bitrate) => {
                    check(sys::aacEncoder_SetParam(handle.ptr, sys::AACENC_PARAM_AACENC_BITRATE, bitrate))?;
                    0
                }
                BitRate::VbrVeryLow => 1,
                BitRate::VbrLow => 2,
                BitRate::VbrMedium => 3,
                BitRate::VbrHigh => 4,
                BitRate::VbrVeryHigh => 5,
            };

            check(sys::aacEncoder_SetParam(handle.ptr, sys::AACENC_PARAM_AACENC_BITRATEMODE, bitrate_mode))?;

            check(sys::aacEncoder_SetParam(handle.ptr, sys::AACENC_PARAM_AACENC_SAMPLERATE, params.sample_rate))?;

            check(sys::aacEncoder_SetParam(handle.ptr, sys::AACENC_PARAM_AACENC_TRANSMUX, match params.transport {
                Transport::Adts => 2,
                Transport::Raw => 0,
            }))?;

            // hardcode SBR off for now
            check(sys::aacEncoder_SetParam(handle.ptr, sys::AACENC_PARAM_AACENC_SBR_MODE, 0))?;

            // hardcode stereo
            check(sys::aacEncoder_SetParam(handle.ptr, sys::AACENC_PARAM_AACENC_CHANNELMODE, 2))?;

            // call encode once with all null params according to docs
            check(sys::aacEncEncode(handle.ptr, ptr::null(), ptr::null(), ptr::null(), ptr::null_mut()))?;
        }

        Ok(Encoder { handle })
    }

    pub fn info(&self) -> Result<InfoStruct, EncoderError> {
        let mut info = MaybeUninit::uninit();
        check(unsafe { sys::aacEncInfo(self.handle.ptr, info.as_mut_ptr()) })?;
        Ok(unsafe { info.assume_init() })
    }

    pub fn encode<R: Read, W: Write>(&self, input: &mut R, output: &mut W) -> Result<EncodeInfo, EncoderError> {

        let info = self.info()?;

        let channels = 2; // hard-coded to stereo
        let buffer_len = 2*channels*info.frameLength as usize;
        let mut input_buffer = vec![0; buffer_len];
        let mut output_buffer = vec![0; buffer_len];

        let mut total_consumed_samples = 0;
        let mut total_written_bytes = 0;
        loop {
            let input_len = input.read(&mut input_buffer)?;
            if input_len == 0 {
                break;
            }

            let mut input_buf = input_buffer.as_ptr() as *mut i16;
            let mut input_buf_ident: c_int = sys::AACENC_BufferIdentifier_IN_AUDIO_DATA as c_int;
            let mut input_buf_size: c_int = input_len as c_int;
            let mut input_buf_el_size: c_int = mem::size_of::<i16>() as c_int;
            let input_desc = sys::AACENC_BufDesc {
                numBufs: 1,
                bufs: &mut input_buf as *mut _ as *mut *mut c_void,
                bufferIdentifiers: &mut input_buf_ident as *mut c_int,
                bufSizes: &mut input_buf_size as *mut c_int,
                bufElSizes: &mut input_buf_el_size as *mut c_int,
            };

            let mut output_buf = output_buffer.as_mut_ptr();
            let mut output_buf_ident: c_int = sys::AACENC_BufferIdentifier_OUT_BITSTREAM_DATA as c_int;
            let mut output_buf_size: c_int = output_buffer.len() as c_int;
            let mut output_buf_el_size: c_int = mem::size_of::<i16>() as c_int;
            let output_desc = sys::AACENC_BufDesc {
                numBufs: 1,
                bufs: &mut output_buf as *mut _ as *mut *mut c_void,
                bufferIdentifiers: &mut output_buf_ident as *mut _,
                bufSizes: &mut output_buf_size as *mut _,
                bufElSizes: &mut output_buf_el_size as *mut _,
            };

            let in_args = sys::AACENC_InArgs {
                numInSamples: input_len as i32 / 2,
                numAncBytes: 0,
            };

            let mut out_args = unsafe { mem::zeroed() };

            let code = unsafe {
                sys::aacEncEncode(
                    self.handle.ptr,
                    &input_desc,
                    &output_desc,
                    &in_args,
                    &mut out_args,
                )
            };

            if code != sys::AACENC_ERROR_AACENC_OK {
                if code == sys::AACENC_ERROR_AACENC_ENCODE_EOF {
                    break;
                }

                return Err(EncoderError::FdkAac(code));
            }

            let input_consumed = out_args.numInSamples as usize;
            let output_size = out_args.numOutBytes as usize;
            output.write(&output_buffer[0..output_size])?;
            total_consumed_samples += input_consumed;
            total_written_bytes += output_size;
        }

        Ok(EncodeInfo {
            output_size: total_written_bytes,
            input_consumed: total_consumed_samples,
        })
    }
}

impl Debug for Encoder {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Encoder {{ handle: {:?} }}", self.handle.ptr)
    }
}
