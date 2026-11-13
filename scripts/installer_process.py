"""Bounded owned-process cancellation for the explicit installer command."""

from __future__ import annotations
import os
import signal
import subprocess
import sys


class OwnedRunner:
    def __call__(self, argv, **options):
        interactive = options.pop("interactive", False)
        checked = options.pop("check", False)
        timeout = options.pop("timeout", None)
        data = options.pop("input", None)
        capture = options.pop("capture_output", False)
        if capture:
            options.update(stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        if data is not None:
            options["stdin"] = subprocess.PIPE
        terminal = sys.stdin.fileno() if interactive else None
        foreground = os.tcgetpgrp(terminal) if interactive else None
        process = subprocess.Popen(
            argv,
            process_group=0 if interactive else None,
            start_new_session=not interactive,
            **options,
        )
        try:
            try:
                if interactive:
                    self.foreground(terminal, process.pid)
                stdout, stderr = process.communicate(input=data, timeout=timeout)
            except BaseException:
                # Kill only this operation's process group. Native services already
                # installed through systemd are retained for explicit recovery.
                try:
                    os.killpg(process.pid, signal.SIGTERM)
                except ProcessLookupError:
                    pass
                try:
                    process.communicate(timeout=10)
                except subprocess.TimeoutExpired:
                    try:
                        os.killpg(process.pid, signal.SIGKILL)
                    except ProcessLookupError:
                        pass
                    process.communicate()
                self.cleanup_group(process.pid)
                raise
            # A build command may exit while leaving a background child in its
            # group. End those children before promoting service code as root.
            self.cleanup_group(process.pid)
            result = subprocess.CompletedProcess(
                argv, process.returncode, stdout, stderr
            )
            if checked:
                result.check_returncode()
            return result
        finally:
            if interactive:
                self.foreground(terminal, foreground)

    @staticmethod
    def cleanup_group(group):
        try:
            os.killpg(group, signal.SIGTERM)
        except ProcessLookupError:
            return
        # The direct child was reaped by communicate(); any remaining members
        # are descendants owned by this invocation, not unrelated services.
        try:
            os.killpg(group, signal.SIGKILL)
        except ProcessLookupError:
            pass

    @staticmethod
    def foreground(fd, group):
        previous = signal.signal(signal.SIGTTOU, signal.SIG_IGN)
        try:
            os.tcsetpgrp(fd, group)
        finally:
            signal.signal(signal.SIGTTOU, previous)


def interrupted(_signal, _frame):
    raise InterruptedError("Setup was interrupted; review its private recovery journal")
