import { useQuery } from '@tanstack/react-query';
import { useReadContract } from 'wagmi';
import { lyricsFlipAbi } from '../abi/lyricsFlip';
import { CONTRACT_ADDRESS } from '../config';

export interface GameData {
  lyric: string;
  options: string[];
  round: number;
  stats: {
    totalGames: number;
    totalPlayers: number;
    totalWins: number;
  };
}

/**
 * Reads the current game state from the LyricsFlip contract.
 *
 * Wrapped in React Query so on-chain reads are cached and retried,
 * which keeps the UI stable while RPC calls resolve on testnet.
 */
export function useGameData() {
  const { data: rawGame, isLoading: isGameLoading, error: gameError } = useReadContract({
    address: CONTRACT_ADDRESS,
    abi: lyricsFlipAbi,
    functionName: 'getCurrentGame',
  });

  const { data: rawStats, isLoading: isStatsLoading, error: statsError } = useReadContract({
    address: CONTRACT_ADDRESS,
    abi: lyricsFlipAbi,
    functionName: 'getStats',
  });

  const gameQuery = useQuery({
    queryKey: ['game', 'current', rawGame?.toString()],
    queryFn: async (): Promise<GameData | null> => {
      if (!rawGame) return null;
      const [lyric, options, round] = rawGame as [string, string[], bigint];
      return {
        lyric,
        options,
        round: Number(round),
        stats: {
          totalGames: 0,
          totalPlayers: 0,
          totalWins: 0,
        },
      };
    },
    enabled: !isGameLoading && !!rawGame,
    staleTime: 30_000,
    retry: 3,
  });

  const statsQuery = useQuery({
    queryKey: ['game', 'stats', rawStats?.toString()],
    queryFn: async () => {
      if (!rawStats) return null;
      const [totalGames, totalPlayers, totalWins] = rawStats as [bigint, bigint, bigint];
      return {
        totalGames: Number(totalGames),
        totalPlayers: Number(totalPlayers),
        totalWins: Number(totalWins),
      };
    },
    enabled: !isStatsLoading && !!rawStats,
    staleTime: 30_000,
    retry: 3,
  });

  const data: GameData | null = gameQuery.data
    ? { ...gameQuery.data, stats: statsQuery.data ?? gameQuery.data.stats }
    : null;

  return {
    data,
    isLoading: isGameLoading || isStatsLoading || gameQuery.isLoading || statsQuery.isLoading,
    error: gameError ?? statsError ?? gameQuery.error ?? statsQuery.error,
    refetch: () => {
      gameQuery.refetch();
      statsQuery.refetch();
    },
  };
}
