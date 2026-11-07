//! Speaker clustering and voiceprint matching on L2-normalised embeddings.

/// Cosine similarity of two L2-normalised vectors.
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

pub fn normalize(v: &mut [f32]) {
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if n > 0.0 {
        v.iter_mut().for_each(|x| *x /= n);
    }
}

/// Mean of vectors, re-normalised.
pub fn centroid(vs: &[&[f32]]) -> Vec<f32> {
    let dim = vs.first().map_or(0, |v| v.len());
    let mut c = vec![0f32; dim];
    for v in vs {
        for (i, x) in v.iter().enumerate() {
            c[i] += x;
        }
    }
    normalize(&mut c);
    c
}

/// Agglomerative clustering with average linkage on cosine distance.
/// Stops when the closest pair of clusters is farther apart than
/// `max_distance`, or, when `target` is given, when that many clusters are
/// left. Then folds clusters holding less than `min_share` of the total
/// weight into their nearest larger cluster (stray segments, laughter,
/// cross-talk). `weights` are e.g. segment durations. Returns one label per
/// input, numbered by first appearance.
pub fn cluster(embeddings: &[Vec<f32>], weights: &[f32], max_distance: f32, target: Option<usize>, min_share: f32) -> Vec<usize> {
    let n = embeddings.len();
    if n == 0 {
        return Vec::new();
    }
    // Members of each live cluster.
    let mut clusters: Vec<Option<Vec<usize>>> = (0..n).map(|i| Some(vec![i])).collect();
    // Pairwise average distances between live clusters.
    let mut dist = vec![vec![0f32; n]; n];
    for i in 0..n {
        for j in i + 1..n {
            let d = 1.0 - cosine(&embeddings[i], &embeddings[j]);
            dist[i][j] = d;
            dist[j][i] = d;
        }
    }
    let mut live = n;
    loop {
        if let Some(t) = target {
            if live <= t.max(1) {
                break;
            }
        }
        let mut best: Option<(usize, usize, f32)> = None;
        for i in 0..n {
            if clusters[i].is_none() {
                continue;
            }
            for j in i + 1..n {
                if clusters[j].is_none() {
                    continue;
                }
                if best.map_or(true, |(_, _, d)| dist[i][j] < d) {
                    best = Some((i, j, dist[i][j]));
                }
            }
        }
        let Some((a, b, d)) = best else { break };
        if target.is_none() && d > max_distance {
            break;
        }
        // Merge b into a; update average-linkage distances.
        let (na, nb) = (clusters[a].as_ref().unwrap().len() as f32, clusters[b].as_ref().unwrap().len() as f32);
        for k in 0..n {
            if k == a || k == b || clusters[k].is_none() {
                continue;
            }
            let d = (dist[a][k] * na + dist[b][k] * nb) / (na + nb);
            dist[a][k] = d;
            dist[k][a] = d;
        }
        let mb = clusters[b].take().unwrap();
        clusters[a].as_mut().unwrap().extend(mb);
        live -= 1;
    }

    let mut groups: Vec<Vec<usize>> = clusters.into_iter().flatten().collect();
    // Fold tiny clusters into the nearest big one (by centroid).
    let total: f32 = weights.iter().sum::<f32>().max(f32::EPSILON);
    let share = |g: &Vec<usize>| g.iter().map(|&i| weights[i]).sum::<f32>() / total;
    if target.is_none() && groups.len() > 1 {
        let (big, small): (Vec<Vec<usize>>, Vec<Vec<usize>>) = groups.into_iter().partition(|g| share(g) >= min_share);
        if big.is_empty() {
            groups = small;
        } else {
            groups = big;
            let cents: Vec<Vec<f32>> = groups.iter().map(|g| centroid(&g.iter().map(|&i| embeddings[i].as_slice()).collect::<Vec<_>>())).collect();
            for g in small {
                let c = centroid(&g.iter().map(|&i| embeddings[i].as_slice()).collect::<Vec<_>>());
                let nearest = (0..groups.len()).max_by(|&x, &y| cosine(&c, &cents[x]).total_cmp(&cosine(&c, &cents[y]))).unwrap();
                groups[nearest].extend(g);
            }
        }
    }
    // Number labels by first appearance.
    groups.sort_by_key(|g| *g.iter().min().unwrap());
    let mut labels = vec![0usize; n];
    for (label, g) in groups.iter().enumerate() {
        for &i in g {
            labels[i] = label;
        }
    }
    labels
}

/// Greedy one-to-one matching of speaker centroids to known voiceprints.
/// Returns, per speaker, the index of the matched profile if its similarity
/// is at least `min_similarity`.
pub fn match_profiles(speakers: &[Vec<f32>], profiles: &[Vec<f32>], min_similarity: f32) -> Vec<Option<usize>> {
    let mut pairs: Vec<(usize, usize, f32)> = Vec::new();
    for (s, sv) in speakers.iter().enumerate() {
        for (p, pv) in profiles.iter().enumerate() {
            if sv.len() == pv.len() {
                let c = cosine(sv, pv);
                if c >= min_similarity {
                    pairs.push((s, p, c));
                }
            }
        }
    }
    pairs.sort_by(|a, b| b.2.total_cmp(&a.2));
    let mut out = vec![None; speakers.len()];
    let mut used = vec![false; profiles.len()];
    for (s, p, _) in pairs {
        if out[s].is_none() && !used[p] {
            out[s] = Some(p);
            used[p] = true;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Unit vector near axis `k` with a small deterministic wobble.
    fn v(k: usize, wobble: usize) -> Vec<f32> {
        let mut x = vec![0f32; 8];
        x[k] = 1.0;
        x[(k + 1 + wobble % 3) % 8] = 0.15 * ((wobble % 5) as f32 + 1.0) / 5.0;
        normalize(&mut x);
        x
    }

    #[test]
    fn finds_speakers_by_threshold_and_numbers_by_appearance() {
        let e = vec![v(3, 0), v(3, 1), v(0, 2), v(3, 3), v(5, 4), v(0, 5), v(5, 6)];
        let w = vec![1.0; e.len()];
        assert_eq!(cluster(&e, &w, 0.5, None, 0.0), vec![0, 0, 1, 0, 2, 1, 2]);
    }

    #[test]
    fn target_count_forces_merges() {
        let e = vec![v(3, 0), v(0, 1), v(5, 2), v(3, 3)];
        let l = cluster(&e, &[1.0; 4], 0.5, Some(2), 0.0);
        assert_eq!(l.iter().copied().collect::<std::collections::HashSet<_>>().len(), 2);
        assert_eq!(l[0], l[3]);
    }

    #[test]
    fn tiny_clusters_fold_into_nearest() {
        let mut stray = v(3, 0);
        stray[6] = 0.9; // its own cluster at 0.5, but closest to speaker 3
        normalize(&mut stray);
        let e = vec![v(3, 0), v(3, 1), v(0, 2), v(0, 3), stray];
        let w = vec![5.0, 5.0, 5.0, 5.0, 0.2];
        let l = cluster(&e, &w, 0.3, None, 0.05);
        assert_eq!(l, vec![0, 0, 1, 1, 0]);
    }

    #[test]
    fn matches_profiles_one_to_one() {
        let speakers = vec![v(0, 0), v(3, 0), v(6, 0)];
        let profiles = vec![v(3, 1), v(0, 1)];
        assert_eq!(match_profiles(&speakers, &profiles, 0.8), vec![Some(1), Some(0), None]);
        assert_eq!(match_profiles(&speakers, &[v(0, 0), v(0, 1)], 0.8), vec![Some(0), None, None]);
    }

    #[test]
    fn empty_input() {
        assert!(cluster(&[], &[], 0.5, None, 0.0).is_empty());
    }
}
