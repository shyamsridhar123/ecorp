use super::*;
use crony_domain::{MissionContext, MissionOrigin};

impl PgStore {
    /// Read one mission's exact Factory relationship under the viewer's current
    /// Corp, human-operator and room membership. This never reads claim tokens.
    pub async fn mission_context(
        &self,
        corp_id: Uuid,
        viewer_actor_id: Uuid,
        mission_id: Uuid,
    ) -> Result<Option<MissionContext>> {
        // Factory materialization commits the mission and its UNIQUE mission_id
        // link together. No supported path adopts or unlinks a committed mission;
        // reset_demo removes both aggregates in one transaction. Therefore this
        // complete inverse lookup can distinguish direct from Factory work.
        //
        // Keep authorization and linkage in the same statement/snapshot. A
        // filtered/absent viewer is None, never an invented Direct result.
        let row = sqlx::query(
            r#"
            SELECT mission.id AS mission_id, mission.corp_id, mission.room_id,
                   item.id AS work_item_id,
                   item.source_repository_owner, item.source_repository_name,
                   item.source_issue_number, item.source_issue_url
            FROM missions mission
            JOIN rooms room
              ON room.id = mission.room_id AND room.corp_id = mission.corp_id
            JOIN room_memberships membership
              ON membership.room_id = room.id AND membership.actor_id = $2
            JOIN actors viewer
              ON viewer.id = membership.actor_id AND viewer.corp_id = mission.corp_id
             AND viewer.kind = 'human'
             AND viewer.role IN ('owner', 'admin', 'manager', 'member')
            LEFT JOIN factory_work_items item
              ON item.mission_id = mission.id AND item.corp_id = mission.corp_id
            WHERE mission.corp_id = $1 AND mission.id = $3
              AND NOT EXISTS (
                  SELECT 1 FROM factory_work_items invalid_link
                  WHERE invalid_link.mission_id = mission.id
                    AND invalid_link.corp_id <> mission.corp_id
              )
            "#,
        )
        .bind(corp_id)
        .bind(viewer_actor_id)
        .bind(mission_id)
        .fetch_optional(&self.pool)
        .await?;

        row.map(|row| {
            let origin = match row.try_get::<Option<Uuid>, _>("work_item_id")? {
                Some(work_item_id) => MissionOrigin::Factory {
                    work_item_id,
                    source_repository: format!(
                        "{}/{}",
                        row.try_get::<String, _>("source_repository_owner")?,
                        row.try_get::<String, _>("source_repository_name")?
                    ),
                    source_issue_number: row.try_get("source_issue_number")?,
                    source_issue_url: row.try_get("source_issue_url")?,
                },
                None => MissionOrigin::Direct,
            };
            Ok(MissionContext {
                corp_id: row.try_get("corp_id")?,
                actor_id: viewer_actor_id,
                mission_id: row.try_get("mission_id")?,
                room_id: row.try_get("room_id")?,
                origin,
            })
        })
        .transpose()
    }
}
