import asyncio


async def resolve_tier_placeholders(value):
    from app.lol.connector import connector

    icons = {}
    names = {}
    semaphore = asyncio.Semaphore(2)

    async def resolve(item):
        if isinstance(item, str):
            if item.startswith("@champion_name:"):
                champion_id = int(item.split(":", 1)[1])
                if champion_id not in names:
                    names[champion_id] = connector.manager.getChampionNameById(
                        champion_id
                    )
                return names[champion_id]
            if item.startswith("@champion:"):
                champion_id = int(item.split(":", 1)[1])
                if champion_id not in icons:
                    async with semaphore:
                        icons[champion_id] = await connector.getChampionIcon(
                            champion_id
                        )
                return icons[champion_id]
            return item
        if isinstance(item, list):
            return await asyncio.gather(*(resolve(entry) for entry in item))
        if isinstance(item, dict):
            return {key: await resolve(entry) for key, entry in item.items()}
        return item

    return await resolve(value)


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
        data = await self.client.opgg_tier_list(region, mode, tier)
        return await resolve_tier_placeholders(data)

    async def getChampionPositions(self, region, championId, tier, mode="ranked"):
        return await self.client.opgg_positions(
            region, championId, tier, mode
        )
