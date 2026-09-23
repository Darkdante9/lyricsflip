// Numeric contract error codes, mirroring `onchain/contracts/lyricsflip/src/errors.rs`
// and the `Error` enum in `onchain/contracts/lyricsflip-nft/src/lib.rs`. The
// codes are stable (see the table in `onchain/README.md`); keep this file in
// sync when a variant is added.

export interface ContractErrorInfo {
  name: string;
  message: string;
}

export const LYRICSFLIP_ERRORS: Record<number, ContractErrorInfo> = {
  1: { name: 'AlreadyInitialized', message: 'The contract is already initialized.' },
  2: { name: 'NonExistingRound', message: 'That round does not exist.' },
  3: { name: 'RoundAlreadyStarted', message: 'That round has already started.' },
  4: { name: 'NonExistingGenre', message: 'Pick a genre to create a round.' },
  5: { name: 'RoundAlreadyJoined', message: 'You have already joined this round.' },
  6: { name: 'InvalidCardsPerRound', message: 'Cards per round must be greater than zero.' },
  7: { name: 'ArtistCardsIsZero', message: 'There are no cards for that artist yet.' },
  8: { name: 'EmptyYearCards', message: 'There are no cards for that year yet.' },
  9: { name: 'EmptyGenreCards', message: 'There are no cards for that genre yet.' },
  10: { name: 'RoundNotStarted', message: 'That round has not started yet.' },
  11: { name: 'RoundCompleted', message: 'That round is already finished.' },
  12: { name: 'NotAParticipant', message: 'You are not a player in this round.' },
  13: { name: 'AlreadyReady', message: 'You are already marked as ready.' },
  14: { name: 'NotAuthorized', message: 'You are not authorized to do that.' },
  15: { name: 'AmountExceedsLimit', message: 'Not enough cards to build a round.' },
  16: { name: 'LimitMustBeGreaterThanZero', message: 'No cards have been added yet.' },
  17: { name: 'NonExistingCard', message: 'That card does not exist.' },
};

export const LYRICSFLIP_NFT_ERRORS: Record<number, ContractErrorInfo> = {
  1: { name: 'AlreadyInitialized', message: 'The NFT contract is already initialized.' },
  2: { name: 'NotMinter', message: 'Only the minter can mint rewards.' },
  3: { name: 'TokenAlreadyExists', message: 'That token has already been minted.' },
  4: { name: 'TokenDoesNotExist', message: 'That token does not exist.' },
};

/**
 * Extracts the numeric code from a Soroban contract error, which surfaces in
 * simulation/submission messages as `Error(Contract, #<code>)`.
 */
export function getContractErrorCode(error: unknown): number | undefined {
  const text = error instanceof Error ? error.message : String(error);
  const match = /Error\(Contract, #(\d+)\)/.exec(text);
  return match ? Number(match[1]) : undefined;
}

export function describeContractError(
  error: unknown,
  table: Record<number, ContractErrorInfo> = LYRICSFLIP_ERRORS,
): ContractErrorInfo | undefined {
  const code = getContractErrorCode(error);
  return code === undefined ? undefined : table[code];
}
