import importlib.util
import sys
import tempfile
import unittest
from pathlib import Path


BENCH_DIR = Path(__file__).resolve().parents[1] / "bench"
if str(BENCH_DIR) not in sys.path:
    sys.path.insert(0, str(BENCH_DIR))
SPEC = importlib.util.spec_from_file_location("frame_time_ab", BENCH_DIR / "frame_time_ab.py")
frame_time_ab = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(frame_time_ab)


class FrameTimeAnalysisTests(unittest.TestCase):
    def test_reads_and_summarizes_presentmon_csv(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "presentmon.csv"
            path.write_text(
                "CPUStartQPCTime,MsBetweenPresents\n"
                "1000,16.7\n1016.7,20\n1036.7,60\n1096.7,NA\n",
                encoding="utf-8",
            )
            frames = frame_time_ab.read_presentmon(path)
        summary = frame_time_ab.summarize_frames(frames)
        self.assertEqual(summary["frames"], 3)
        self.assertEqual(summary["stallsOver50Ms"], 1)
        self.assertGreater(summary["onePercentLowFps"], 0)

    def test_correlates_stall_with_sidecar_samples(self):
        frames = [
            {"qpcTimeMs": 1000, "frameTimeMs": 16.7},
            {"qpcTimeMs": 1020, "frameTimeMs": 60},
        ]
        samples = [{"qpcTimeMs": 1019, "cpuPercent": 75}]
        requests = [{"qpcTimeMs": 1021, "latencyMs": 42}]
        result = frame_time_ab.correlate_stalls(frames, samples, requests)
        self.assertEqual(result["stallFrames"], 1)
        self.assertEqual(result["sidecarCpuMaxPercent"], 75)
        self.assertEqual(result["nearbyRequestMaxMs"], 42)

    def test_aggregates_multiple_rounds(self):
        rounds = []
        for p99, stalls in [(30, 6), (32, 9), (31, 3)]:
            rounds.append({
                "frames": {
                    "frameTimeP50Ms": 10,
                    "frameTimeP95Ms": 20,
                    "frameTimeP99Ms": p99,
                    "stallsOver50Ms": stalls,
                },
                "sidecar": {
                    "durationSeconds": 60,
                    "cpuAvgPercent": 1,
                    "latencyP99Ms": 0.5,
                    "workingSetAvgMB": 5,
                },
            })
        result = frame_time_ab.aggregate_rounds(rounds)
        self.assertEqual(result["rounds"], 3)
        self.assertEqual(result["frameTimeP99Ms"], 31)
        self.assertEqual(result["frameTimeP99WorstMs"], 32)
        self.assertEqual(result["stallsOver50MsPerMinute"], 6)

    def test_regression_thresholds_pass_and_fail(self):
        baseline = {"frameTimeP99Ms": 30, "stallsOver50MsPerMinute": 1}
        passed = frame_time_ab.compare_regression(
            {"frameTimeP99Ms": 31, "stallsOver50MsPerMinute": 1.5}, baseline
        )
        self.assertTrue(passed["passed"])
        failed = frame_time_ab.compare_regression(
            {"frameTimeP99Ms": 33, "stallsOver50MsPerMinute": 4}, baseline
        )
        self.assertFalse(failed["passed"])
        self.assertEqual(len(failed["reasons"]), 2)

    def test_missing_baseline_p99_is_not_accepted(self):
        result = frame_time_ab.compare_regression(
            {"frameTimeP99Ms": 20, "stallsOver50MsPerMinute": 0},
            {"stallsOver50MsPerMinute": 0},
        )
        self.assertFalse(result["passed"])


if __name__ == "__main__":
    unittest.main()
