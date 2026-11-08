use clap::{Parser, ValueEnum};
use std::{fmt::Write, process::ExitCode};

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
type ResourceLong = i64;
#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
type ResourceLong = std::ffi::c_long;

#[cfg(all(target_os = "linux", target_env = "musl"))]
type TimePart = i64;
#[cfg(not(all(target_os = "linux", target_env = "musl")))]
type TimePart = ResourceLong;

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum Who {
    #[value(name = "self")]
    ThisProcess,
    Children,
}

impl Who {
    fn name(self) -> &'static str {
        match self {
            Self::ThisProcess => "self",
            Self::Children => "children",
        }
    }
}

#[derive(Parser, Debug)]
#[command(version, about = "Summarise Linux process resource usage")]
struct Args {
    #[arg(
        long,
        value_enum,
        default_value = "self",
        help = "Select this process or terminated children already waited for"
    )]
    who: Who,
    #[arg(
        long,
        help = "Print JSON with stable field names and units in the field names"
    )]
    json: bool,
}

#[derive(Clone, Copy, Default)]
#[repr(C)]
struct Timeval {
    seconds: TimePart,
    microseconds: TimePart,
}

impl Timeval {
    fn formatted(self) -> String {
        format!("{}.{:06}", self.seconds, self.microseconds)
    }
}

#[derive(Default)]
#[repr(C)]
struct ResourceUsage {
    user_time: Timeval,
    system_time: Timeval,
    max_rss: ResourceLong,
    shared_memory: ResourceLong,
    unshared_data: ResourceLong,
    unshared_stack: ResourceLong,
    minor_faults: ResourceLong,
    major_faults: ResourceLong,
    swaps: ResourceLong,
    block_input: ResourceLong,
    block_output: ResourceLong,
    messages_sent: ResourceLong,
    messages_received: ResourceLong,
    signals: ResourceLong,
    voluntary_switches: ResourceLong,
    involuntary_switches: ResourceLong,
    reserved: [ResourceLong; 16],
}

#[cfg(target_os = "linux")]
fn collect(who: Who) -> Result<ResourceUsage, String> {
    // Linux libc ABI: C-layout rusage prefix with reserved space for libc extensions.
    unsafe extern "C" {
        fn getrusage(who: std::ffi::c_int, usage: *mut ResourceUsage) -> std::ffi::c_int;
    }
    let selector = match who {
        Who::ThisProcess => 0,
        Who::Children => -1,
    };
    let mut usage = ResourceUsage::default();
    // The pointer is aligned, writable and valid for the duration of the call.
    if unsafe { getrusage(selector, &mut usage) } != 0 {
        return Err(format!(
            "getrusage failed: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(usage)
}

#[cfg(not(target_os = "linux"))]
fn collect(_: Who) -> Result<ResourceUsage, String> {
    Err("unsupported platform: getrusage requires Linux".to_owned())
}

fn render(who: Who, usage: &ResourceUsage, json: bool) -> String {
    let user = usage.user_time.formatted();
    let system = usage.system_time.formatted();
    let counters = [
        (
            "max_resident_set_size_kib",
            "Maximum resident set size (KiB)",
            usage.max_rss,
        ),
        (
            "minor_page_faults_count",
            "Minor page faults",
            usage.minor_faults,
        ),
        (
            "major_page_faults_count",
            "Major page faults",
            usage.major_faults,
        ),
        (
            "block_input_operations_count",
            "Block input operations",
            usage.block_input,
        ),
        (
            "block_output_operations_count",
            "Block output operations",
            usage.block_output,
        ),
        (
            "voluntary_context_switches_count",
            "Voluntary context switches",
            usage.voluntary_switches,
        ),
        (
            "involuntary_context_switches_count",
            "Involuntary context switches",
            usage.involuntary_switches,
        ),
    ];
    let mut output = String::new();
    if json {
        write!(output, "{{\"who\":\"{}\",\"user_cpu_time_seconds\":{user},\"system_cpu_time_seconds\":{system}", who.name()).unwrap();
        for (field, _, value) in counters {
            write!(output, ",\"{field}\":{value}").unwrap();
        }
        output.push_str("}\n");
    } else {
        writeln!(
            output,
            "Who: {}\nUser CPU time (seconds): {user}\nSystem CPU time (seconds): {system}",
            who.name()
        )
        .unwrap();
        for (_, label, value) in counters {
            writeln!(output, "{label}: {value}").unwrap();
        }
    }
    output
}

fn main() -> ExitCode {
    let args = Args::parse();
    match collect(args.who) {
        Ok(usage) => {
            use std::io::Write;
            match std::io::stdout()
                .lock()
                .write_all(render(args.who, &usage, args.json).as_bytes())
            {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => {
                    eprintln!("getrusage: could not write output: {error}");
                    ExitCode::FAILURE
                }
            }
        }
        Err(error) => {
            eprintln!("getrusage: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selectors_are_validated_and_default_to_self() {
        assert_eq!(
            Args::try_parse_from(["getrusage"]).unwrap().who,
            Who::ThisProcess
        );
        let args = Args::try_parse_from(["getrusage", "--who", "children", "--json"]).unwrap();
        assert_eq!(args.who, Who::Children);
        assert!(args.json);
        assert!(Args::try_parse_from(["getrusage", "--who", "unknown"]).is_err());
    }

    #[test]
    fn json_has_stable_fields_units_and_exact_cpu_precision() {
        let usage = ResourceUsage {
            user_time: Timeval {
                seconds: 12,
                microseconds: 34,
            },
            system_time: Timeval {
                seconds: 0,
                microseconds: 999999,
            },
            max_rss: 1024,
            minor_faults: 2,
            major_faults: 3,
            block_input: 4,
            block_output: 5,
            voluntary_switches: 6,
            involuntary_switches: 7,
            ..ResourceUsage::default()
        };
        assert_eq!(
            render(Who::Children, &usage, true),
            concat!(
                "{\"who\":\"children\",\"user_cpu_time_seconds\":12.000034,",
                "\"system_cpu_time_seconds\":0.999999,\"max_resident_set_size_kib\":1024,",
                "\"minor_page_faults_count\":2,\"major_page_faults_count\":3,",
                "\"block_input_operations_count\":4,\"block_output_operations_count\":5,",
                "\"voluntary_context_switches_count\":6,\"involuntary_context_switches_count\":7}\n"
            )
        );
    }

    #[test]
    fn text_labels_include_cpu_and_memory_units() {
        let output = render(Who::ThisProcess, &ResourceUsage::default(), false);
        assert!(output.starts_with("Who: self\nUser CPU time (seconds): 0.000000\n"));
        assert!(output.contains("Maximum resident set size (KiB): 0\n"));
        assert!(output.ends_with("Involuntary context switches: 0\n"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_collects_both_selectors() {
        for who in [Who::ThisProcess, Who::Children] {
            let usage = collect(who).unwrap();
            assert!(usage.user_time.seconds >= 0);
            assert!((0..1_000_000).contains(&usage.user_time.microseconds));
            assert!(usage.max_rss >= 0);
        }
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn other_platforms_return_a_clear_error() {
        assert_eq!(
            collect(Who::ThisProcess).err().unwrap(),
            "unsupported platform: getrusage requires Linux"
        );
    }
}
