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


if __name__ == "__main__":
    unittest.main()
