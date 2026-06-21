#include "runtime/ps2_audio.h"
#include "runtime/ps2_memory.h"
#include "ps2_host_backend.h"
#include <algorithm>
#include <cmath>
#include <cstring>
#include <vector>

namespace ps2_vag
{
    bool decode(const uint8_t *data, uint32_t sizeBytes,
                std::vector<int16_t> &outPcm, uint32_t &outSampleRate);
}

PS2AudioBackend::PS2AudioBackend() = default;

PS2AudioBackend::~PS2AudioBackend()
{
    stopAll();
}

void PS2AudioBackend::onVagTransfer(const uint8_t *rdram, uint32_t srcAddr, uint32_t sizeBytes)
{
    if (!rdram || sizeBytes < 48)
        return;

    const uint32_t physAddr = srcAddr & PS2_RAM_MASK;
    if (physAddr + sizeBytes > PS2_RAM_SIZE)
        return;

    std::vector<int16_t> pcm;
    uint32_t sampleRate = 44100;
    if (!ps2_vag::decode(rdram + physAddr, sizeBytes, pcm, sampleRate))
        return;

    std::lock_guard<std::mutex> lock(m_mutex);
    DecodedSample sample;
    sample.pcm = std::move(pcm);
    sample.sampleRate = sampleRate;
    m_sampleBank[physAddr] = std::move(sample);
    m_mostRecentSampleKey = physAddr;
}

void PS2AudioBackend::onVagTransferFromBuffer(const uint8_t *data, uint32_t sizeBytes, uint32_t keyAddr)
{
    if (!data || sizeBytes < 48)
        return;

    std::vector<int16_t> pcm;
    uint32_t sampleRate = 44100;
    if (!ps2_vag::decode(data, sizeBytes, pcm, sampleRate))
        return;

    const uint32_t physAddr = keyAddr & PS2_RAM_MASK;
    std::lock_guard<std::mutex> lock(m_mutex);
    DecodedSample sample;
    sample.pcm = std::move(pcm);
    sample.sampleRate = sampleRate;
    m_sampleBank[physAddr] = sample;
    m_mostRecentSampleKey = physAddr;
    m_loadOrderSamples.push_back(std::move(sample));
    m_loadOrderSampleKeys.push_back(physAddr);
    constexpr size_t kMaxLoadOrderSamples = 32;
    if (m_loadOrderSamples.size() > kMaxLoadOrderSamples)
    {
        m_loadOrderSamples.erase(m_loadOrderSamples.begin());
        m_loadOrderSampleKeys.erase(m_loadOrderSampleKeys.begin());
    }
}

namespace
{
    constexpr uint32_t LIBSD_CMD_SET_VOICE = 0x8010u;
}

void PS2AudioBackend::onSoundCommand(uint32_t sid, uint32_t rpcNum,
                                     const uint8_t *sendBuf, uint32_t sendSize,
                                     uint8_t *recvBuf, uint32_t recvSize)
{
    if (sid != 0x80000701u)
        return;

    if ((rpcNum == LIBSD_CMD_SET_VOICE || (rpcNum & 0xFF00u) == 0x8100u) &&
        sendBuf && sendSize >= 20)
    {
        uint32_t sampleAddr = 0;
        uint32_t voiceIndex = 0xFFFFFFFFu;
        for (int vo = 4; vo >= 0 && voiceIndex == 0xFFFFFFFFu; vo -= 4)
        {
            if (vo < static_cast<int>(sendSize))
            {
                uint32_t v = 0;
                std::memcpy(&v, sendBuf + vo, sizeof(v));
                if (v < 24u)
                    voiceIndex = v;
            }
        }

        constexpr uint32_t kMinPlausibleAddr = 0x1000u;
        for (int off = 12; off <= 24 && sampleAddr == 0; off += 4)
        {
            if (sendSize >= static_cast<uint32_t>(off + 4))
            {
                uint32_t cand = 0;
                std::memcpy(&cand, sendBuf + off, sizeof(cand));
                if (cand >= kMinPlausibleAddr && (cand <= PS2_RAM_MASK || (cand & ~PS2_RAM_MASK) == 0))
                    sampleAddr = cand;
            }
        }
        if (sampleAddr == 0)
            sampleAddr = m_mostRecentSampleKey;

        float pitch = 1.0f;
        if (sendSize >= 12)
        {
            uint16_t pitchHalf = 0;
            std::memcpy(&pitchHalf, sendBuf + 8, sizeof(pitchHalf));
            if (pitchHalf != 0)
                pitch = 4096.0f / static_cast<float>(pitchHalf);
        }
        play(sampleAddr, pitch, 1.0f, voiceIndex);
    }
}

void PS2AudioBackend::play(uint32_t sampleAddr, float pitch, float volume, uint32_t voiceIndex)
{
    std::lock_guard<std::mutex> lock(m_mutex);
    DecodedSample *sampleToPlay = nullptr;

    auto it = m_sampleBank.find(sampleAddr & PS2_RAM_MASK);
    if (it != m_sampleBank.end())
    {
        sampleToPlay = &it->second;
    }
    else if (voiceIndex != 0xFFFFFFFFu &&
             voiceIndex < m_loadOrderSamples.size() &&
             voiceIndex < m_loadOrderSampleKeys.size())
    {
        sampleToPlay = &m_loadOrderSamples[voiceIndex];
    }
    else
    {
        it = m_sampleBank.find(m_mostRecentSampleKey);
        if (it == m_sampleBank.end())
            return;
        sampleToPlay = &it->second;
    }
    if (!sampleToPlay || sampleToPlay->pcm.empty())
        return;

    submitDecodedSample(*sampleToPlay, pitch, volume);
}

// Resample a decoded mono sample to the host output rate, apply pitch + volume,
// expand to interleaved stereo, and push it to the host audio stream.
//
// NOTE (tracked gap): this is a per-sample submit, not a true SPU2 voice mixer.
// Concurrent voices are summed by the host stream's queue rather than mixed
// sample-accurately, and ADSR/loop points are not honored. The real continuous
// PCM seam is wired (ps2_host_audio_submit); the full 24-voice SPU2 mixer is a
// separately tracked core-side task. See notes/documentation/host-seam.md A.3.
void PS2AudioBackend::submitDecodedSample(const DecodedSample &sample, float pitch, float volume)
{
    if (!m_host || !m_audioReady || sample.pcm.empty())
        return;

    const double srcRate = static_cast<double>(sample.sampleRate) * std::max(0.01f, pitch);
    const double dstRate = static_cast<double>(kOutputSampleRate);
    const double step = srcRate / dstRate;
    const size_t srcFrames = sample.pcm.size();
    const size_t dstFrames = static_cast<size_t>(static_cast<double>(srcFrames) / step);
    if (dstFrames == 0)
        return;

    std::vector<int16_t> out(dstFrames * kOutputChannels);
    const float vol = std::clamp(volume, 0.0f, 1.0f);
    double srcPos = 0.0;
    for (size_t i = 0; i < dstFrames; ++i)
    {
        // Linear interpolation between adjacent source samples.
        const size_t idx = static_cast<size_t>(srcPos);
        const double frac = srcPos - static_cast<double>(idx);
        const int16_t a = sample.pcm[std::min(idx, srcFrames - 1)];
        const int16_t b = sample.pcm[std::min(idx + 1, srcFrames - 1)];
        const double mixed = (static_cast<double>(a) * (1.0 - frac) + static_cast<double>(b) * frac) * vol;
        const int16_t s = static_cast<int16_t>(std::clamp(mixed, -32768.0, 32767.0));
        out[i * kOutputChannels + 0] = s;
        out[i * kOutputChannels + 1] = s;
        srcPos += step;
    }

    ps2_host_audio_submit(m_host, out.data(), static_cast<uint32_t>(dstFrames));
}

void PS2AudioBackend::stop(uint32_t voiceId)
{
    (void)voiceId;
}

void PS2AudioBackend::stopAll()
{
    std::lock_guard<std::mutex> lock(m_mutex);
    if (m_host)
        ps2_host_audio_clear(m_host);
}
