/*
 * Copyright (C) 2023 INRIA
 * Copyright (C) 2023 Sebastiano Vigna
 *
 * SPDX-License-Identifier: LGPL-2.1-or-later OR Apache-2.0
 */

/*!
A tunable progress logger to log progress information about long-running activities.
It is a port of the Java class [`it.unimi.dsi.util.ProgressLogger`](https://dsiutils.di.unimi.it/docs/it/unimi/dsi/logging/ProgressLogger.html)
from the [DSI Utilities](https://dsiutils.di.unimi.it/).
Logging is based on the standard [`log`](https://docs.rs/log) crate at the `info` level.

To log the progress of an activity, you call [`start`](#methods.start). Then, each time you want to mark progress,
you call [`update`](#methods.update), which increases the item counter, and will log progress information
if enough time has passed since the last log. The time check happens only on multiples of
[`LIGHT_UPDATE_MASK`](#fields.LIGHT_UPDATE_MASK) + 1 in the case of [`light_update`](#methods.light_update),
which should be used when the activity has an extremely low cost that is comparable to that
of the time check (a call to [`Instant::now()`]) itself.

Some fields can be set at any time to customize the logger: please see the [documentation of the fields](#fields).
It is also possible to log used and free memory at each log interval by calling
[`display_memory`](#methods.display_memory). Memory is read from system data by the [`sysinfo`] crate, and
will be updated at each log interval (note that this will slightly slow down the logging process). Moreover,
since it is impossible to update the memory information from the [`Display::fmt`] implementation,
you should call [`refresh_memory`](#methods.refresh_memory) before displaying the logger
on your own.

At any time, displaying the progress logger will give you time information up to the present.
When the activity is over, you call [`stop`](#methods.stop), which fixes the final time, and
possibly display again the logger. [`done`](#methods.done) will stop the logger, print `Completed.`,
and display the final stats. There are also a few other utility methods that make it possible to
customize the logging process.

After you finished a run of the progress logger, can call [`start`](#fields.start)
again to measure another activity.

A typical call sequence to a progress logger is as follows:
```
use dsi_progress_logger::ProgressLogger;

stderrlog::new().init().unwrap();
let mut pl = ProgressLogger::default();
pl.item_name = "pumpkin".to_string();
pl.start("Smashing pumpkins...");
for _ in 0..100 {
   // do something on each pumlkin
   pl.update();
}
pl.done();
```
A progress logger can also be used as a handy timer:
```
use dsi_progress_logger::ProgressLogger;

stderrlog::new().init().unwrap();
let mut pl = ProgressLogger::default();
pl.item_name = "pumpkin".to_string();
pl.start("Smashing pumpkins...");
for _ in 0..100 {
   // do something on each pumlkin
}
pl.done_with_count(100);
```
This progress logger will display information about  memory usage:
```
use dsi_progress_logger::ProgressLogger;

stderrlog::new().init().unwrap();
let mut pl = ProgressLogger::default().display_memory();
```
*/
use log::info;
use num_format::{Locale, ToFormattedString};
use pluralizer::pluralize;
use std::fmt::{Display, Formatter, Result};
use std::time::{Duration, Instant};
use sysinfo::{Pid, ProcessExt, RefreshKind, System, SystemExt};

mod utils;
use utils::*;

pub struct ProgressLogger {
    /// The name of an item. Defaults to `item`.
    pub item_name: String,
    /// The log interval. Defaults to 10 seconds.
    pub log_interval: Duration,
    /// The expected number of updates. If set, the logger will display the percentage of completion and
    /// an estimate of the time to completion.
    pub expected_updates: Option<usize>,
    /// The time unit to use for speed. If set, the logger will always display the speed in this unit
    /// instead of making a choice of readable unit based on the elapsed time. Moreover, large numbers
    /// will not be thousands separated. This is useful when the output of the logger must be parsed.
    pub time_unit: Option<TimeUnit>,
    /// Display additionally the speed achieved during the last log interval.
    pub local_speed: bool,
    start_time: Option<Instant>,
    last_log_time: Instant,
    next_log_time: Instant,
    stop_time: Option<Instant>,
    count: usize,
    last_count: usize,
    /// Display additionally the amount of used and free memory using this [`sysinfo::System`]
    system: Option<System>,
    /// The pid of the current process
    pid: Pid,
}

impl Default for ProgressLogger {
    fn default() -> Self {
        Self {
            item_name: "item".to_string(),
            log_interval: Duration::from_secs(10),
            expected_updates: None,
            time_unit: None,
            local_speed: false,
            start_time: None,
            last_log_time: Instant::now(),
            next_log_time: Instant::now(),
            stop_time: None,
            count: 0,
            last_count: 0,
            system: None,
            pid: Pid::from(std::process::id() as usize),
        }
    }
}

impl ProgressLogger {
    /// Calls to [light_update](#method.light_update) will cause a call to
    /// [`Instant::now`] only if the current count
    /// is a multiple of this mask plus one.
    pub const LIGHT_UPDATE_MASK: usize = (1 << 20) - 1;
    /// Start the logger, displaying the given message.
    pub fn start<T: AsRef<str>>(&mut self, msg: T) {
        let now = Instant::now();
        self.start_time = Some(now);
        self.stop_time = None;
        self.count = 0;
        self.last_count = 0;
        self.last_log_time = now;
        self.next_log_time = now + self.log_interval;
        info!("{}", msg.as_ref());
    }

    /// Chainable setter enabling memory display.
    pub fn display_memory(mut self) -> Self {
        if self.system.is_none() {
            self.system = Some(System::new_with_specifics(RefreshKind::new().with_memory()));
        }
        self
    }

    /// Refresh memory information, if previously requested with [`display_memory`](#methods.display_memory).
    /// You do not need to call this method unless you display the logger manually.
    pub fn refresh(&mut self) {
        if let Some(system) = &mut self.system {
            system.refresh_memory();
            system.refresh_process(self.pid);
        }
    }

    fn log(&mut self, now: Instant) {
        self.refresh();
        info!("{}", self);
        self.last_count = self.count;
        self.last_log_time = now;
        self.next_log_time = now + self.log_interval;
    }

    fn log_if(&mut self) {
        let now = Instant::now();
        if self.next_log_time <= now {
            self.log(now);
        }
    }

    /// Increase the count and check whether it is time to log.
    pub fn update(&mut self) {
        self.count += 1;
        self.log_if();
    }

    /// Set the count and check whether it is time to log.
    pub fn update_with_count(&mut self, count: usize) {
        self.count += count;
        self.log_if();
    }

    /// Increase the count and, once every [`LIGHT_UPDATE_MASK`](#fields.LIGHT_UPDATE_MASK) + 1 calls, check whether it is time to log.
    pub fn light_update(&mut self) {
        self.count += 1;
        if (self.count & Self::LIGHT_UPDATE_MASK) == 0 {
            self.log_if();
        }
    }

    /// Increase the count and force a log.
    pub fn update_and_display(&mut self) {
        self.count += 1;
        self.log(Instant::now());
    }

    /// Stop the logger, fixing the final time.
    pub fn stop(&mut self) {
        self.stop_time = Some(Instant::now());
        self.expected_updates = None;
    }

    /// Stop the logger, print `Completed.`, and display the final stats. The number of expected updates will be cleared.
    pub fn done(&mut self) {
        self.stop();
        info!("Completed.");
        // just to avoid wrong reuses
        self.expected_updates = None;
        info!("{}", self);
    }

    /// Stop the logger, set the count, print `Completed.`, and display the final stats.
    /// The number of expected updates will be cleared.
    ///
    /// This method is particularly useful in two circumstances:
    /// * you have updated the logger with some approximate values (e.g., in a multicore computation) but before
    ///   printing the final stats you want the internal counter to contain an exact value;
    /// * you have used the logger as a handy timer, calling just [`start`](#fields.start) and this method.

    pub fn done_with_count(&mut self, count: usize) {
        self.count = count;
        self.done();
    }

    /// Return the elapsed time since the logger was started, or `None` if the logger has not been started.
    pub fn elapsed(&self) -> Option<Duration> {
        self.start_time?.elapsed().into()
    }

    fn fmt_timing_speed(&self, f: &mut Formatter<'_>, seconds_per_item: f64) -> Result {
        let items_per_second = 1.0 / seconds_per_item;

        let time_unit_timing = self
            .time_unit
            .unwrap_or_else(|| TimeUnit::nice_time_unit(seconds_per_item));

        let time_unit_speed = self
            .time_unit
            .unwrap_or_else(|| TimeUnit::nice_speed_unit(seconds_per_item));

        f.write_fmt(format_args!(
            "{:.2} {}/{}, {:.2} {}/{}",
            seconds_per_item / time_unit_timing.as_seconds(),
            time_unit_timing.label(),
            self.item_name,
            items_per_second * time_unit_speed.as_seconds(),
            pluralize(&self.item_name, 2, false),
            time_unit_speed.label()
        ))?;

        Ok(())
    }
}

impl Display for ProgressLogger {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        if let Some(start_time) = self.start_time {
            let count_fmtd = if self.time_unit.is_none() {
                self.count.to_formatted_string(&Locale::en)
            } else {
                self.count.to_string()
            };

            if let Some(stop_time) = self.stop_time {
                let elapsed = stop_time - start_time;
                let seconds_per_item = elapsed.as_secs_f64() / self.count as f64;

                f.write_fmt(format_args!(
                    "Elapsed: {}",
                    TimeUnit::pretty_print(elapsed.as_millis())
                ))?;

                if self.count != 0 {
                    f.write_fmt(format_args!(
                        " [{} {}, ",
                        count_fmtd,
                        pluralize(&self.item_name, self.count as isize, false)
                    ))?;
                    self.fmt_timing_speed(f, seconds_per_item)?;
                    f.write_fmt(format_args!("]"))?
                }
            } else {
                let now = Instant::now();

                let elapsed = now - start_time;

                f.write_fmt(format_args!(
                    "{} {}, {}, ",
                    count_fmtd,
                    pluralize(&self.item_name, self.count as isize, false),
                    TimeUnit::pretty_print(elapsed.as_millis()),
                ))?;

                let seconds_per_item = elapsed.as_secs_f64() / self.count as f64;
                self.fmt_timing_speed(f, seconds_per_item)?;

                if let Some(expected_updates) = self.expected_updates {
                    let millis_to_end: u128 = ((expected_updates - self.count) as u128
                        * elapsed.as_millis())
                        / (self.count as u128 + 1);
                    f.write_fmt(format_args!(
                        "; {:.2}% done, {} to end",
                        100.0 * self.count as f64 / expected_updates as f64,
                        TimeUnit::pretty_print(millis_to_end)
                    ))?;
                }

                if self.local_speed && self.stop_time.is_none() {
                    f.write_fmt(format_args!(" ["))?;

                    let elapsed = now - self.last_log_time;
                    let seconds_per_item =
                        elapsed.as_secs_f64() / (self.count - self.last_count) as f64;
                    self.fmt_timing_speed(f, seconds_per_item)?;

                    f.write_fmt(format_args!("]"))?;
                }
            }

            if let Some(system) = &self.system {
                f.write_fmt(format_args!(
                    "; used/avail/free/total mem {}B/{}B/{}B/{}B",
                    system
                        .process(self.pid)
                        .map(|process| humanize(process.memory() as _))
                        .unwrap_or("N/A".to_string()),
                    humanize(system.available_memory() as _),
                    humanize(system.free_memory() as _),
                    humanize(system.total_memory() as _)
                ))?;
            }

            Ok(())
        } else {
            write!(f, "ProgressLogger not started")
        }
    }
}
