class RustOpgg:
    def __init__(self, client):
        self.client = client
        self.proxy = None
        self.session = None
        self.assetSession = None

    async def start(self):
        await self.client.start()

    async def close(self):
        await self.client.close()

    async def getClassicRuneIcon(self, url, sourceToken):
        return await self.client.asset_get(
            "classic rune icons",
            f"{sourceToken}.png",
            url=url,
        )

    async def getLegacyMasteryIcon(self, url, masteryId):
        return await self.client.asset_get(
            "legacy mastery icons",
            f"{masteryId}.png",
            url=url,
        )

    async def getChampionBuild(self, region, mode, championId, position, tier):
        return await self.client.opgg_champion_build(
            region, mode, championId, position, tier
        )

    async def getTierList(self, region, mode, tier):
        return await self.client.opgg_tier_list(region, mode, tier)

    async def getChampionPositions(self, region, championId, tier, mode="ranked"):
        return await self.client.opgg_positions(
            region, championId, tier, mode
        )
