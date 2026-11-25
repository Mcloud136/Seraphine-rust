from .exceptions import SummonerGamesNotFound


def extract_match_history_games(result):
    games = result.get("games")
    if isinstance(games, dict):
        games = games.get("games")
    if not isinstance(games, list):
        raise SummonerGamesNotFound()
    return games
