use super::sys;

const AVCODEC: Library = Library {
    name: "libavcodec",
    built: version(
        sys::LIBAVCODEC_VERSION_MAJOR,
        sys::LIBAVCODEC_VERSION_MINOR,
        sys::LIBAVCODEC_VERSION_MICRO,
    ),
};

const AVUTIL: Library = Library {
    name: "libavutil",
    built: version(
        sys::LIBAVUTIL_VERSION_MAJOR,
        sys::LIBAVUTIL_VERSION_MINOR,
        sys::LIBAVUTIL_VERSION_MICRO,
    ),
};

struct Library {
    name: &'static str,
    built: u32,
}

const fn version(major: u32, minor: u32, micro: u32) -> u32 {
    (major << 16) | (minor << 8) | micro
}

fn format(version: u32) -> String {
    format!("{}.{}.{}", version >> 16, (version >> 8) & 0xff, version & 0xff)
}

impl Library {
    fn check(&self, loaded: u32) -> Result<(), String> {
        if loaded >> 16 == self.built >> 16 && loaded >= self.built {
            return Ok(());
        }
        Err(format!(
            "{} {} was built against, but {} is loaded, and the two do not have the same structure layouts",
            self.name,
            format(self.built),
            format(loaded)
        ))
    }
}

pub fn check() -> Result<(), String> {
    AVCODEC.check(unsafe { sys::avcodec_version() })?;
    AVUTIL.check(unsafe { sys::avutil_version() })
}

pub fn loaded() -> String {
    format!(
        "{} {}, {} {}",
        AVCODEC.name,
        format(unsafe { sys::avcodec_version() }),
        AVUTIL.name,
        format(unsafe { sys::avutil_version() })
    )
}

#[cfg(test)]
mod tests {
    use super::Library;
    use super::version;

    const LIBRARY: Library = Library {
        name: "libavcodec",
        built: version(62, 28, 102),
    };

    #[test]
    fn the_version_it_was_built_against_is_accepted() {
        assert!(LIBRARY.check(version(62, 28, 102)).is_ok());
    }

    #[test]
    fn a_newer_version_of_the_same_major_is_accepted() {
        assert!(LIBRARY.check(version(62, 29, 0)).is_ok());
    }

    #[test]
    fn an_older_version_of_the_same_major_is_rejected() {
        assert!(LIBRARY.check(version(62, 27, 100)).is_err());
    }

    #[test]
    fn another_major_version_is_rejected() {
        assert!(LIBRARY.check(version(63, 0, 0)).is_err());
        assert!(LIBRARY.check(version(61, 40, 0)).is_err());
    }
}
