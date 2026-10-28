package inbox

import "github.com/StephenSHorton/irori/internal/blob"

// Attachment finds the file sha on a message reader may read (v0.4 §6.6):
// one it sent, or one with a delivery to it that isn't held. The person
// (an owner key) reads every message. The sha alone is not enough: an
// attach.offered row names it to every device, so the message is the
// permission.
func (b *Inbox) Attachment(reader, sha string) (blob.FileRef, bool) {
	return b.impl.attachment(reader, sha)
}

func (b *inbox) attachment(reader, sha string) (blob.FileRef, bool) {
	b.mu.Lock()
	defer b.unlock()
	person := b.isOwner(reader)
	find := func(m *Message) (blob.FileRef, bool) {
		for _, f := range m.Attachments {
			if f.SHA256 == sha {
				return f, true
			}
		}
		return blob.FileRef{}, false
	}
	for _, m := range b.msgs {
		if m.From.Key == reader || person {
			if f, ok := find(m); ok {
				return f, true
			}
		}
	}
	for _, d := range b.dels {
		if d.To != reader || d.State == Held {
			continue
		}
		if m := b.msgs[d.MessageID]; m != nil {
			if f, ok := find(m); ok {
				return f, true
			}
		}
	}
	return blob.FileRef{}, false
}
