export async function fetchBlob(url: string): Promise<Uint8Array> {
    const res = await fetch(url);
    if (!res.ok) throw new Error(`Fetch failed (${res.status})`);
    return new Uint8Array(await res.arrayBuffer());
}
