use std::io::Error;
use tokio::select;
#[cfg(not(target_os = "windows"))]
use tokio::signal::unix::{Signal, SignalKind, signal};
#[cfg(target_os = "windows")]
use tokio::signal::windows::{
    CtrlBreak, CtrlC, CtrlClose, CtrlLogoff, CtrlShutdown, ctrl_break, ctrl_c, ctrl_close,
    ctrl_logoff, ctrl_shutdown,
};

#[cfg(not(target_os = "windows"))]
pub struct TerminationSignals {
    terminate: Signal,
    interrupt: Signal,
    quit: Signal,
    alarm: Signal,
    hangup: Signal,
}

#[cfg(not(target_os = "windows"))]
impl TerminationSignals {
    pub fn new() -> Result<Self, Error> {
        Ok(Self {
            terminate: signal(SignalKind::terminate())?,
            interrupt: signal(SignalKind::interrupt())?,
            quit: signal(SignalKind::quit())?,
            alarm: signal(SignalKind::alarm())?,
            hangup: signal(SignalKind::hangup())?,
        })
    }

    pub async fn recv(&mut self) {
        select! {
            _ = self.terminate.recv() => (),
            _ = self.interrupt.recv() => (),
            _ = self.quit.recv() => (),
            _ = self.alarm.recv() => (),
            _ = self.hangup.recv() => (),
        }
    }
}

#[cfg(target_os = "windows")]
pub struct TerminationSignals {
    ctrl_break: CtrlBreak,
    ctrl_c: CtrlC,
    ctrl_close: CtrlClose,
    ctrl_logoff: CtrlLogoff,
    ctrl_shutdown: CtrlShutdown,
}

#[cfg(target_os = "windows")]
impl TerminationSignals {
    pub fn new() -> Result<Self, Error> {
        Ok(Self {
            ctrl_break: ctrl_break()?,
            ctrl_c: ctrl_c()?,
            ctrl_close: ctrl_close()?,
            ctrl_logoff: ctrl_logoff()?,
            ctrl_shutdown: ctrl_shutdown()?,
        })
    }

    pub async fn recv(&mut self) {
        select! {
            _ = self.ctrl_break.recv() => (),
            _ = self.ctrl_c.recv() => (),
            _ = self.ctrl_close.recv() => (),
            _ = self.ctrl_logoff.recv() => (),
            _ = self.ctrl_shutdown.recv() => (),
        }
    }
}
