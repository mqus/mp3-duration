#[macro_use]
extern crate failure;

use failure::Error;
use std::io::prelude::*;
use std::io::SeekFrom;
use std::fs::File;
use std::path::Path;
use std::time::Duration;

#[derive(Debug, Fail)]
enum MP3DurationError {
    #[fail(display = "Invalid MPEG version")]
    ForbiddenVersion,
    #[fail(display = "Invalid MPEG Layer (0)")]
    ForbiddenLayer,
    #[fail(display = "Invalid bitrate bits: {}", bitrate)]
    InvalidBitrate { bitrate: u8, },
    #[fail(display = "Invalid sampling rate bits: {}", sampling_rate)]
    InvalidSamplingRate { sampling_rate: u8, },
    #[fail(display = "Unexpected frame, header: {}", header)]
    UnexpectedFrame {
        header: u32,
    }
}

#[derive(Clone, Copy, Debug)]
enum Version {
    Mpeg1,
    Mpeg2,
    Mpeg25,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Layer {
    NotDefined,
    Layer1,
    Layer2,
    Layer3,
}

static BIT_RATES: [[[u32; 16]; 4]; 3] = [[
        [0;16],
        [0, 32, 64, 96, 128, 160, 192, 224, 256, 288, 320, 352, 384, 416, 448, 0], // Mpeg1 Layer1
        [0, 32, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 384, 0],    // Mpeg1 Layer2
        [0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 0],     // Mpeg1 Layer3
    ],
                                         [
        [0;16],
        [0, 32, 48, 56, 64, 80, 96, 112, 128, 144, 160, 176, 192, 224, 256, 0],   // Mpeg2 Layer1
        [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160, 0],        // Mpeg2 Layer2
        [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160, 0],        // Mpeg2 Layer3
    ],
                                         [
        [0;16],
        [0, 32, 48, 56, 64, 80, 96, 112, 128, 144, 160, 176, 192, 224, 256, 0],   // Mpeg25 Layer1
        [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160, 0],        // Mpeg25 Layer2
        [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160, 0],        // Mpeg25 Layer3
    ]];


static SAMPLING_RATES: [[u32; 4]; 3] = [
    [44100, 48000, 32000, 0],   // Mpeg1
    [22050, 24000, 16000, 0],   // Mpeg2
    [11025, 12000, 8000, 0],    // Mpeg25
];

static SAMPLES_PER_FRAME: [[u32; 4]; 3] = [
    [0, 384, 1152, 1152],   // Mpeg1
    [0, 384, 1152, 576],    // Mpeg2
    [0, 384, 1152, 576],    // Mpeg25
];

fn get_bitrate(version: Version, layer: Layer, encoded_bitrate: u8) -> Result<u32, Error> {
    if encoded_bitrate <= 0 || encoded_bitrate >= 15 {
        bail!(MP3DurationError::InvalidBitrate{ bitrate: encoded_bitrate });
    }
    if layer == Layer::NotDefined {
        bail!(MP3DurationError::ForbiddenLayer);
    }
    Ok(1000 * BIT_RATES[version as usize][layer as usize][encoded_bitrate as usize])
}

fn get_sampling_rate(version: Version, encoded_sampling_rate: u8) -> Result<u32, Error> {
    if encoded_sampling_rate >= 3 {
        bail!(MP3DurationError::InvalidSamplingRate{ sampling_rate: encoded_sampling_rate });
    }
    Ok(SAMPLING_RATES[version as usize][encoded_sampling_rate as usize])
}

fn get_samples_per_frame(version: Version, layer: Layer) -> Result<u32, Error> {
    if layer == Layer::NotDefined {
        bail!(MP3DurationError::ForbiddenLayer);
    }
    Ok(SAMPLES_PER_FRAME[version as usize][layer as usize])
}

/// Measures the duration of a file.
///
/// # Examples
///
/// ```
/// use std::path::Path;
/// use std::fs::File;
/// use mp3_duration;
///
/// let path = Path::new("test/source.mp3");
/// let mut file = File::open(path).unwrap();
/// let duration = mp3_duration::from_file(&mut file).unwrap();
/// println!("File duration: {:?}", duration);
/// ```
pub fn from_file<T>(file: &mut T) -> Result<Duration, Error>
    where T: Read + Seek
{
    let mut buffer = [0; 4];

    let mut duration = Duration::from_secs(0);
    loop {
        match file.read_exact(&mut buffer[..]) {
            Ok(_) => (),
            Err(e) => {
                match e.kind() {
                    std::io::ErrorKind::UnexpectedEof => break,
                    _ => bail!(e),
                }
            }
        };

        // ID3v1 frame
        let is_id3v1 = buffer[0] == 'T' as u8 && buffer[1] == 'A' as u8 && buffer[2] == 'G' as u8;
        if is_id3v1 {
            file.seek(SeekFrom::Current(124))?; // 4 bytes already read
            continue;
        }

        // ID3v2 frame
        let is_id3v2 = buffer[0] == 'I' as u8 && buffer[1] == 'D' as u8 && buffer[2] == '3' as u8;
        if is_id3v2 {
            let mut id3v2 = [0; 6]; // 4 bytes already read
            file.read_exact(&mut id3v2)?;
            let flags = id3v2[1];
            let footer_size = if 0 != (flags & 0b00010000) { 10 } else { 0 };
            let tag_size = (id3v2[5] as u32) | ((id3v2[4] as u32) << 7) |
                           ((id3v2[3] as u32) << 14) |
                           ((id3v2[2] as u32) << 21);
            file.seek(SeekFrom::Current(tag_size as i64 + footer_size))?;
            continue;
        }

        // MPEG frame
        let header = (buffer[0] as u32) << 24 | (buffer[1] as u32) << 16 |
                     (buffer[2] as u32) << 8 | buffer[3] as u32;
        let is_mp3 = header >> 21 == 0x7FF;
        if is_mp3 {

            let version = match (header >> 19) & 0b11 {
                0 => Version::Mpeg25,
                1 => bail!(MP3DurationError::ForbiddenVersion),
                2 => Version::Mpeg2,
                3 => Version::Mpeg1,
                _ => unreachable!(),
            };

            let layer = match (header >> 17) & 0b11 {
                0 => Layer::NotDefined,
                1 => Layer::Layer3,
                2 => Layer::Layer2,
                3 => Layer::Layer1,
                _ => unreachable!(),
            };

            let encoded_bitrate = (header >> 12) & 0b1111;
            let encoded_sampling_rate = (header >> 10) & 0b11;
            let padding = if 0 != ((header >> 9) & 1) { 1 } else { 0 };
            let bitrate = get_bitrate(version, layer, encoded_bitrate as u8)?;
            let sampling_rate = get_sampling_rate(version, encoded_sampling_rate as u8)?;
            let num_samples = get_samples_per_frame(version, layer)?;
            let frame_duration = (num_samples as u64 * 1_000_000_000) / (sampling_rate as u64);
            let frame_length = num_samples / 8 * bitrate / sampling_rate + padding - 4;

            file.seek(SeekFrom::Current(frame_length as i64))?;
            duration = duration + Duration::new(0, frame_duration as u32);
            continue;
        }

        bail!(MP3DurationError::UnexpectedFrame{ header: header });
    }

    Ok(duration)
}

/// Measures the duration of a file.
///
/// # Examples
///
/// ```
/// use std::path::Path;
/// use mp3_duration;
///
/// let path = Path::new("test/source.mp3");
/// let duration = mp3_duration::from_path(&path).unwrap();
/// println!("File duration: {:?}", duration);
/// ```
pub fn from_path<P>(path: P) -> Result<Duration, Error>
    where P: AsRef<Path>
{
    let mut file = File::open(path)?;
    from_file(&mut file)
}

#[test]
fn constant_bitrate_320() {
    let path = Path::new("test/CBR320.mp3");
    let duration = from_path(path).unwrap();
    assert_eq!(398, duration.as_secs());
}

#[test]
fn variable_bitrate_v0() {
    let path = Path::new("test/VBR0.mp3");
    let duration = from_path(path).unwrap();
    assert_eq!(398, duration.as_secs());
}

#[test]
fn variable_bitrate_v9() {
    let path = Path::new("test/VBR9.mp3");
    let duration = from_path(path).unwrap();
    assert_eq!(398, duration.as_secs());
}

#[test]
fn id3v1() {
    let path = Path::new("test/ID3v1.mp3");
    let duration = from_path(path).unwrap();
    assert_eq!(398, duration.as_secs());
}

#[test]
fn id3v2() {
    let path = Path::new("test/ID3v2.mp3");
    let duration = from_path(path).unwrap();
    assert_eq!(398, duration.as_secs());
}

#[test]
fn bad_file() {
    let path = Path::new("test/piano.jpeg");
    let duration = from_path(path);
    assert!(duration.is_err());
}
