import unittest
from collections import deque

from PyQt5.QtWidgets import QApplication

from app.view.search_interface import GamesTab


class SearchPaginationTests(unittest.IsolatedAsyncioTestCase):
    @classmethod
    def setUpClass(cls):
        cls.app = QApplication.instance() or QApplication([])

    @staticmethod
    def game(index):
        return {
            "gameId": index,
            "remake": False,
            "win": True,
            "championIcon": "app/resource/images/champion-0.png",
            "name": "Test",
            "kills": 1,
            "deaths": 2,
            "assists": 3,
            "shortTime": "10:00",
        }

    def setUp(self):
        self.tab = GamesTab()
        self.tab.games = deque(self.game(index) for index in range(10))
        self.tab.queueIdMap = {-1: list(range(10))}
        self.tab.currentPageNum = 0
        self.tab.maxPageNum = 0
        self.tab.showTheFirstPage()
        self.calls = 0

        async def load_more():
            self.calls += 1
            start = len(self.tab.games)
            self.tab.games.extend(self.game(index) for index in range(start, start + 10))
            self.tab.queueIdMap[-1].extend(range(start, start + 10))

        self.tab.loadMoreCallback = load_more

    def tearDown(self):
        self.tab.close()
        self.tab.deleteLater()

    async def test_next_page_loads_one_batch_on_demand(self):
        await self.tab._GamesTab__onNextButtonClicked()
        self.assertEqual(self.calls, 1)
        self.assertEqual(self.tab.currentPageNum, 2)
        self.assertEqual(self.tab.maxPageNum, 2)
        self.assertEqual(len(self.tab.games), 20)
        self.assertTrue(self.tab.nextButton.isEnabled())

    async def test_exhausted_history_does_not_request_more(self):
        self.tab.hasMore = False
        await self.tab._GamesTab__onNextButtonClicked()
        self.assertEqual(self.calls, 0)
        self.assertEqual(self.tab.currentPageNum, 1)
        self.assertFalse(self.tab.nextButton.isEnabled())


if __name__ == "__main__":
    unittest.main()
