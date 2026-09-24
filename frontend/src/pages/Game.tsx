import { useEffect, useMemo, useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { useParams } from 'react-router-dom';
import { useWallet } from '../hooks/useWallet';
import { useGameContract } from '../hooks/useGameContract';
import { LyricCard } from '../components/LyricCard';
import { StatisticsPanel } from '../components/StatisticsPanel';
import { SongOptions } from '../components/SongOptions';
import { RoundInfo } from '../components/RoundInfo';
import { Skeleton } from '../components/Skeleton';
import type { Game, Round } from '../types';

const GAME_QUERY_KEY = 'game';
const ROUND_QUERY_KEY = 'round';

export function GamePage() {
  const { gameId } = useParams<{ gameId: string }>();
  const { address } = useWallet();
  const { fetchGame, fetchRound } = useGameContract();

  const {
    data: game,
    isLoading: isGameLoading,
    isError: isGameError,
  } = useQuery<Game>({
    queryKey: [GAME_QUERY_KEY, gameId],
    queryFn: () => fetchGame(gameId as string),
    enabled: Boolean(gameId),
    staleTime: 30_000,
    retry: 2,
  });

  const {
    data: round,
    isLoading: isRoundLoading,
    isError: isRoundError,
  } = useQuery<Round>({
    queryKey: [ROUND_QUERY_KEY, gameId, game?.currentRound],
    queryFn: () => fetchRound(gameId as string, game?.currentRound as number),
    enabled: Boolean(gameId) && game?.currentRound !== undefined,
    staleTime: 30_000,
    retry: 2,
  });

  const [selectedSong, setSelectedSong] = useState<string | null>(null);

  useEffect(() => {
    setSelectedSong(null);
  }, [round?.id]);

  const isRoundActive = useMemo(
    () => round?.status === 'active',
    [round?.status],
  );

  if (isGameError || isRoundError) {
    return (
      <div className="game-page game-page--error">
        <p>Failed to load game. Please try again.</p>
      </div>
    );
  }

  return (
    <div className="game-page">
      <header className="game-page__header">
        {isGameLoading ? (
          <Skeleton width="12rem" height="1.75rem" />
        ) : (
          <h1 className="game-page__title">{game?.title ?? 'Game'}</h1>
        )}
      </header>

      <section className="game-page__round-info">
        {isRoundLoading ? (
          <Skeleton width="100%" height="4rem" />
        ) : (
          <RoundInfo round={round} />
        )}
      </section>

      <section className="game-page__lyric-card">
        {isRoundLoading ? (
          <Skeleton width="100%" height="10rem" />
        ) : (
          <LyricCard lyric={round?.lyric} />
        )}
      </section>

      <section className="game-page__song-options">
        {isRoundLoading ? (
          <div className="game-page__song-options-skeleton">
            <Skeleton width="100%" height="3rem" />
            <Skeleton width="100%" height="3rem" />
            <Skeleton width="100%" height="3rem" />
            <Skeleton width="100%" height="3rem" />
          </div>
        ) : (
          <SongOptions
            options={round?.options ?? []}
            selected={selectedSong}
            disabled={!isRoundActive || !address}
            onSelect={setSelectedSong}
          />
        )}
      </section>

      <section className="game-page__statistics">
        {isGameLoading || isRoundLoading ? (
          <Skeleton width="100%" height="6rem" />
        ) : (
          <StatisticsPanel game={game} round={round} />
        )}
      </section>
    </div>
  );
}

export default GamePage;
