"""Actual runner timing boundary with deterministic host serial-read costs."""
import copy
from pathlib import Path
import sys
import unittest

sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
from alpha_session import finish_cycle
from check_alpha_session import cycle_errors


class Clock:
    def __init__(self):self.now=100.0
    def __call__(self):return self.now
    def advance(self,seconds):self.now+=seconds


def legacy_finish(text,clock,cycle,started,cycle_start,serial_start,deadline):
    # Independent reproduction of the old Python left-to-right evaluation:
    # end timestamp, expensive serial read, THEN a new fifteen-second deadline.
    window=dict(cycle=cycle,start_seconds=cycle_start-started,
                end_seconds=clock()-started,serial_start=serial_start,serial_end=len(text()))
    return window,min(deadline,clock()+15)


def session(finisher=finish_cycle,idle=15):
    clock=Clock();started=clock();windows=[];samples=[]
    costs=[.0004*i for i in range(1,85)]
    work=(1800-84*15-sum(costs))/84
    kinds=[('quil_input_receipt',None),('quil_input_pixels',None),
           ('warm_save_receipt',None),('warm_save_pixels',None),('spindle_stat',None),
           ('linen_input_receipt','save_admission'),('linen_refresh','save_admission'),
           ('linen_input_receipt','cycle_end'),('linen_refresh','cycle_end')]
    for cycle,cost in enumerate(costs,1):
        begin=clock();serial_start=(cycle-1)*100
        for index,(kind,phase) in enumerate(kinds):
            action=begin-started+.01*(index+1)
            sample=dict(sample_id=len(samples)+1,cycle=cycle,kind=kind,seconds=.001,
                        action_seconds=action,session_seconds=action+.001,recorded_seconds=action+.002,
                        serial_start=serial_start+index*2,serial_end=serial_start+index*2+1)
            if phase is not None:sample['phase']=phase
            samples.append(sample)
        clock.advance(work)
        def reader():
            clock.advance(cost)
            return 'x'*(cycle*100)
        window,deadline=finisher(reader,clock,cycle,started,begin,serial_start,started+1800)
        windows.append(window)
        if idle==15:clock.now=max(clock(),deadline)
        else:clock.advance(idle)  # Deliberate violation, not scheduler/read overhead.
    elapsed=clock()-started;active=sum(w['end_seconds']-w['start_seconds']for w in windows)
    return dict(cycles=84,elapsed_seconds=elapsed,cycle_windows=windows,samples=samples,
                active_elapsed_seconds=active,idle_elapsed_seconds=elapsed-active)


class CycleTiming(unittest.TestCase):
    def test_actual_helper_reads_once_before_end_and_anchors_idle(self):
        clock=Clock();events=[]
        def read():events.append('read');clock.advance(.25);return 'serial'
        def stamp():events.append('clock');return clock()
        window,deadline=finish_cycle(read,stamp,3,90,99,2,200)
        self.assertEqual(events,['read','clock'])
        self.assertEqual(window,dict(cycle=3,start_seconds=9,end_seconds=10.25,serial_start=2,serial_end=6))
        self.assertEqual(deadline,115.25)
        # Work done after closing the window cannot extend the idle deadline.
        clock.advance(.5);self.assertEqual(deadline-clock(),14.5)
    def test_session_deadline_caps_idle_and_expired_deadline_does_not_move(self):
        clock=Clock()
        _,deadline=finish_cycle(lambda:'x',clock,1,90,99,0,101)
        self.assertEqual(deadline,101)
        _,deadline=finish_cycle(lambda:'x',clock,1,90,99,0,99)
        self.assertEqual(deadline,99)
    def test_growing_read_cost_reproduces_old_failure_corrected_84_cycles_pass(self):
        old=session(legacy_finish);new=session()
        self.assertEqual(cycle_errors(old),['sustained duration contains excessive unaccounted idle time'])
        self.assertGreater(old['idle_elapsed_seconds'],1261)
        self.assertEqual(cycle_errors(new),[])
        self.assertAlmostEqual(new['elapsed_seconds'],1800)
        self.assertAlmostEqual(new['active_elapsed_seconds'],540)
        self.assertAlmostEqual(new['idle_elapsed_seconds'],1260)
        self.assertEqual(len(new['samples']),84*9)
    def test_deliberate_sixteen_second_gaps_still_rejected(self):
        bad=session(idle=16)
        self.assertIn('sustained duration contains excessive unaccounted idle time',cycle_errors(bad))
    def test_missing_reordered_cycle_or_sample_cannot_bypass_validator(self):
        good=session()
        mutations=[lambda s:s['cycle_windows'].pop(40),
                   lambda s:s['cycle_windows'][40].update(cycle=40),
                   lambda s:s['samples'].pop(20),
                   lambda s:s['samples'][20].update(serial_end=s['cycle_windows'][-1]['serial_end']),
                   lambda s:s.update(active_elapsed_seconds=s['elapsed_seconds'],idle_elapsed_seconds=0)]
        for mutation in mutations:
            bad=copy.deepcopy(good);mutation(bad)
            with self.subTest(mutation=mutation):self.assertTrue(cycle_errors(bad))


if __name__=='__main__':unittest.main()
