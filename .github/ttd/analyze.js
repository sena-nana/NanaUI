"use strict";

function initializeScript() {
    return [new host.apiVersionSupport(1, 7)];
}

function log(text) {
    host.diagnostics.debugLog(text + "\n");
}

function exec(command, limit) {
    log("> " + command);
    let count = 0;
    try {
        for (let line of host.namespace.Debugger.Utility.Control.ExecuteCommand(command)) {
            if (limit !== undefined && count++ >= limit) {
                log("  ...");
                break;
            }
            log("  " + line);
        }
    } catch (error) {
        log("  ! " + error);
    }
}

function hex(value) {
    return "0x" + value.toString(16);
}

function before(a, b) {
    let s = a.Sequence.compareTo(b.Sequence);
    return s < 0 || (s == 0 && a.Steps.compareTo(b.Steps) < 0);
}

function invokeScript() {
    let session = host.currentSession;
    let exception = null;
    // Corruption shows as a verifier stop (int3), an access violation or a
    // heap failure; a Rust panic (0xe06d7363) is the script failing instead.
    let wanted = [0x80000003, 0xc0000005, 0xc0000374, 0xc0000409];
    for (let event of host.currentProcess.TTD.Events) {
        if (event.Type == "Exception") {
            let code = event.Exception.Code;
            log("exception event at " + event.Position + " code " + hex(code));
            if (wanted.some((c) => code.compareTo(c) == 0)) {
                exception = event;
            }
        }
    }
    if (exception === null) {
        log("RESULT no corruption exception in trace");
        return;
    }
    exec("!tt " + exception.Position);
    exec(".lastevent");
    exec("r");
    exec("k 40", 60);
    let tid = host.currentThread.Id;
    log("crash thread " + hex(tid));

    // Walk back to the free that found the corrupted header.
    exec("bp ntdll!RtlFreeHeap");
    let block = null;
    for (let i = 0; i < 64 && block === null; i++) {
        exec("g-", 5);
        if (host.currentThread.Id == tid) {
            block = host.currentThread.Registers.User.r8;
        }
    }
    exec("bc *");
    if (block === null) {
        log("RESULT no RtlFreeHeap on the crash thread");
        return;
    }
    let freePosition = host.currentThread.TTD.Position;
    log("RESULT freed block " + hex(block) + " at " + freePosition);
    exec("k 30", 40);
    exec("db " + hex(block.subtract(0x60)) + " L0x80");

    let low = block.subtract(0x80);
    let high = block.add(0x10);
    let writes = [];
    for (let write of session.TTD.Memory(low, high, "w")) {
        if (before(write.TimeStart, freePosition)) {
            writes.push(write);
        }
    }
    writes.sort((a, b) => (before(a.TimeStart, b.TimeStart) ? -1 : before(b.TimeStart, a.TimeStart) ? 1 : 0));
    log("writes to [" + hex(low) + ", " + hex(high) + ") before the free: " + writes.length);
    for (let write of writes.slice(-40)) {
        log("  " + write.TimeStart + " tid " + hex(write.ThreadId) + " ip " + hex(write.IP) +
            " addr " + hex(write.Address) + " size " + write.Size + " value " + hex(write.Value));
    }
    let shown = 0;
    for (let i = writes.length - 1; i >= 0 && shown < 6; i--) {
        let write = writes[i];
        exec("!tt " + write.TimeStart);
        log("WRITER " + write.TimeStart + " tid " + hex(write.ThreadId) + " addr " + hex(write.Address) +
            " size " + write.Size + " value " + hex(write.Value));
        exec("ln " + hex(write.IP));
        exec("u " + hex(write.IP) + " L3");
        exec("r");
        exec("k 50", 60);
        shown++;
    }
}
