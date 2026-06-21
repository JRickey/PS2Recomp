#ifndef PS2_AUDIO_H
#define PS2_AUDIO_H

#include <cstdint>
#include <memory>
#include <mutex>
#include <unordered_map>
#include <vector>

struct PS2Host;

class PS2AudioBackend
{
public:
    PS2AudioBackend();
    ~PS2AudioBackend();

    // Bind the SDL3 host. The output stream must already be opened by the host
    // (ps2_host_audio_open) before samples are submitted.
    void setHost(PS2Host *host) { m_host = host; }

    void onVagTransfer(const uint8_t *rdram, uint32_t srcAddr, uint32_t sizeBytes);
    void onVagTransferFromBuffer(const uint8_t *data, uint32_t sizeBytes, uint32_t keyAddr);
    void onSoundCommand(uint32_t sid, uint32_t rpcNum,
                        const uint8_t *sendBuf, uint32_t sendSize,
                        uint8_t *recvBuf, uint32_t recvSize);

    void play(uint32_t sampleAddr, float pitch = 1.0f, float volume = 1.0f,
              uint32_t voiceIndex = 0xFFFFFFFFu);
    void stop(uint32_t voiceId);
    void stopAll();
    void setAudioReady(bool ready) { m_audioReady = ready; }

    // Output stream format the host was opened with; submitted PCM is resampled
    // to this rate and mixed to this channel count.
    static constexpr uint32_t kOutputSampleRate = 48000u;
    static constexpr uint32_t kOutputChannels = 2u;

private:
    struct DecodedSample
    {
        std::vector<int16_t> pcm;
        uint32_t sampleRate = 44100;
    };

    PS2Host *m_host = nullptr;
    bool m_audioReady = false;
    uint32_t m_mostRecentSampleKey = 0;
    std::vector<DecodedSample> m_loadOrderSamples;
    std::vector<uint32_t> m_loadOrderSampleKeys;
    std::unordered_map<uint32_t, DecodedSample> m_sampleBank;
    std::mutex m_mutex;

    void submitDecodedSample(const DecodedSample &sample, float pitch, float volume);
};

#endif
