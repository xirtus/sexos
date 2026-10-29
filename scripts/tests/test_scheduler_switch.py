"""Negative controls for compiled context release boundary inspection."""
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("switch_check", Path(__file__).resolve().parents[1] / "check_scheduler_switch.py")
checker = importlib.util.module_from_spec(spec)
spec.loader.exec_module(checker)


def valid():
    return ["cli", "mov r12, rsi", "mov rsp, qword ptr [rsi + 0xc0]", "and rsp, -0x10",
            "call 0xffff1234 <scheduler_finish_switch>", "mov rsp, qword ptr [r12 + 0xc0]",
            "wrpkru", "swapgs"] + ["pop " + r for r in
            ("rax", "rbx", "rcx", "rdx", "rbp", "rsi", "rdi", "r8", "r9", "r10", "r11", "r12", "r13", "r14", "r15")] + ["add rsp, 0x8", "iretq"]


class CompiledSwitchTests(unittest.TestCase):
    def test_correct_boundary(self):
        checker.validate(valid())

    def test_pre_stack_release_callback_is_rejected(self):
        code = valid()
        code[2], code[4] = code[4], code[2]
        with self.assertRaisesRegex(ValueError, "RSP pivot"):
            checker.validate(code)

    def test_callback_with_narrowed_pkru_is_rejected(self):
        code = valid()
        code[6] = "nop"
        code.insert(4, "wrpkru")
        with self.assertRaisesRegex(ValueError, "PKRU"):
            checker.validate(code)

    def test_missing_alignment_is_rejected(self):
        code = valid()
        code[3] = "nop"
        with self.assertRaisesRegex(ValueError, "alignment"):
            checker.validate(code)

    def test_changed_frame_or_gpr_layout_is_rejected(self):
        for index in (8, -2):
            code = valid()
            code[index] = "nop"
            with self.assertRaises(ValueError):
                checker.validate(code)

    def test_interrupts_must_be_disabled_before_stack_pivot(self):
        code = valid()
        code[0], code[3] = code[3], code[0]
        with self.assertRaisesRegex(ValueError, "RSP pivot"):
            checker.validate(code)

    def test_disassembly_requires_unique_switch_symbol(self):
        text = "1000 <<sex_kernel::scheduler::Scheduler>::switch_to>:\n1000: cli\n"
        self.assertEqual(checker.switch_instructions(text), ["cli"])
        for bad in ("", text + text):
            with self.assertRaises(ValueError):
                checker.switch_instructions(bad)


if __name__ == "__main__":
    unittest.main()
