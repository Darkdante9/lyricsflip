export interface Song {
  id: number;
  title: string;
  artist: string;
  category: string;
  plays: number;
}

export interface Category {
  id: number;
  name: string;
}

const API_URL = process.env.NEXT_PUBLIC_API_URL ?? '/api';

const getJson = async <T>(path: string): Promise<T> => {
  const response = await fetch(`${API_URL}${path}`);
  if (!response.ok) {
    throw new Error(`Failed to fetch ${path}`);
  }
  return response.json() as Promise<T>;
};

export const fetchSongs = (): Promise<Song[]> => getJson<Song[]>('/songs');

export const fetchCategories = (): Promise<Category[]> =>
  getJson<Category[]>('/categories');
